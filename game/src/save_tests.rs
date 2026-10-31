//! S1: use the same entry points as autosave, the harness and recording.
use super::*;
use crate::grid::ChunkKey;
use crate::world::chunk::VOXELS;
use std::time::Duration;
use crate::world::save::{OsFs, SaveFs, SaveSnapshot, SaveWorker};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, mpsc};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy)]
enum GateAt { Chunk, Published }
enum Release { Write, Fail, Panic }

struct GateFs {
    at: GateAt,
    entered: mpsc::SyncSender<()>,
    release: Mutex<mpsc::Receiver<Release>>,
    used: AtomicBool,
    published: AtomicBool,
    caller: std::thread::ThreadId,
}

impl GateFs {
    fn gate(&self, matches: bool) -> std::io::Result<()> {
        assert_ne!(std::thread::current().id(), self.caller, "disk I/O ran on the caller");
        if matches && !self.used.swap(true, Ordering::SeqCst) {
            self.entered.send(()).unwrap();
            match self.release.lock().unwrap().recv_timeout(Duration::from_secs(20)).unwrap() {
                Release::Write => {},
                Release::Fail => return Err(std::io::Error::other("injected worker write failure")),
                Release::Panic => panic!("injected writer panic"),
            }
        }
        Ok(())
    }
}

impl SaveFs for GateFs {
    fn write_synced(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        self.gate(matches!(self.at, GateAt::Chunk) && path.extension().is_some_and(|e| e == "chunk"))?;
        OsFs.write_synced(path, bytes)
    }
    fn replace_durably(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        self.gate(false)?;
        OsFs.replace_durably(from, to)?;
        if to.file_name().is_some_and(|f| f == "commit.json") { self.published.store(true, Ordering::SeqCst); }
        Ok(())
    }
    fn sync_dir(&self, dir: &Path) -> std::io::Result<()> {
        self.gate(matches!(self.at, GateAt::Published) && self.published.load(Ordering::SeqCst))?;
        OsFs.sync_dir(dir)
    }
    fn remove(&self, path: &Path) -> std::io::Result<()> {
        self.gate(false)?;
        OsFs.remove(path)
    }
}

fn gate(s: &mut Session, at: GateAt) -> (mpsc::Receiver<()>, mpsc::SyncSender<Release>) {
    let (entered, wait) = mpsc::sync_channel(1);
    let (release, resume) = mpsc::sync_channel(1);
    s.save_worker = SaveWorker::with_fs(Arc::new(GateFs {
        at, entered, release: Mutex::new(resume), used: AtomicBool::new(false),
        published: AtomicBool::new(false), caller: std::thread::current().id(),
    }));
    (wait, release)
}

fn fresh(tag: &str) -> Session {
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = SaveDir::new(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("out/s1-tests/{tag}-{}-{nonce}", std::process::id())));
    crate::replay::tests::fresh(Some(dir))
}

fn dirty(s: &Session, n: u32, id: u16) {
    for m in 0..n {
        s.engine.edits.set(ChunkKey::neisor(2, 0, m % 64, m / 64, 17), 1, 2, 3, id, || vec![1; VOXELS]);
    }
}

#[test]
fn s1_autosave_returns_before_acknowledgement() {
    let mut s = fresh("request");
    dirty(&s, 4, 4);
    assert_eq!(s.save_if_due(Instant::now() + Duration::from_secs(20)).unwrap(), 0,
        "starting a save must not report a completed write");
    assert_eq!(s.saved_chunks, 0);
    assert!(s.last_save.is_none());
    assert_eq!(s.edits().dirty_count(), 4);
    s.save_now().unwrap();
}

#[test]
fn s1_saving_preserves_pending_simulation_work_and_dirty_state() {
    let mut s = fresh("pure");
    dirty(&s, 1, 4);
    let bp = BlockPos { face: 2, i: 1, j: 2, k: 547 };
    s.edits().push_changed([bp]);
    s.sim.schedule(bp, 1);
    let before = (s.sim.queued(), s.sim.is_dirty(), s.edits().pending_changes(), s.sim.state_digest(&s.engine.world));
    s.save_now().unwrap();
    let after = (s.sim.queued(), s.sim.is_dirty(), s.edits().pending_changes(), s.sim.state_digest(&s.engine.world));
    assert_eq!(before, after, "save timing must never change simulation or its change log");
}

#[test]
fn s1_recording_reports_a_failed_save_barrier() {
    let mut s = fresh("record-failure");
    let dir = s.save.as_ref().unwrap().path.clone();
    std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
    std::fs::write(&dir, b"not a directory").unwrap();
    dirty(&s, 1, 4);
    let result = s.start_recording(&dir.with_extension("replay"), "must wait for save");
    assert!(result.is_err(), "a failed barrier must reach the caller");
    assert!(!s.recording());
    assert_eq!(s.edits().dirty_count(), 1);
}

#[test]
fn s1_snapshot_is_one_moment_and_busy_requests_allocate_nothing() {
    let mut s = fresh("moment");
    dirty(&s, 2, 4);
    s.sim.set_radius(30.0);
    let before = s.sim.snapshot_for_save(&s.engine.world);
    let (entered, release) = gate(&mut s, GateAt::Chunk);
    assert!(s.request_save().unwrap());
    entered.recv_timeout(Duration::from_secs(20)).unwrap();
    dirty(&s, 3, 5); // two revisions replaced and a new chunk
    s.sim.set_radius(70.0);
    for _ in 0..100 {
        assert!(!s.request_save().unwrap(), "busy request must not capture or queue");
        assert!(s.poll_save().is_none());
    }
    assert_eq!(s.saved_chunks, 0);
    release.send(Release::Write).unwrap();
    assert_eq!(s.wait_for_save().unwrap(), 2);
    let dir = s.save.as_ref().unwrap().clone();
    let back = EditStore::new();
    dir.load_into(&s.engine.config, &s.engine.registry, &s.bake_sha256, &back).unwrap();
    assert_eq!(back.len(), 2);
    assert_eq!(back.get(ChunkKey::neisor(2, 0, 0, 0, 17), 1, 2, 3), Some(4));
    assert_eq!(Snapshot::from_bytes(&dir.read_sim().unwrap().unwrap()).unwrap(), before);
    assert_eq!(s.edits().dirty_count(), 3, "later revisions must survive the acknowledgement");
    s.save_now().unwrap();
    assert_eq!(s.edits().dirty_count(), 0);
    assert_eq!(dir.read_commit().unwrap().unwrap().generation, 2, "no queued writes");
    assert_eq!(Snapshot::from_bytes(&dir.read_sim().unwrap().unwrap()).unwrap().config.unwrap().radius_m, 70.0);
}

#[test]
fn s1_worker_failure_and_panic_keep_dirt_and_retry() {
    for panic in [false, true] {
        let mut s = fresh(if panic { "panic" } else { "write-failure" });
        dirty(&s, 1, 4);
        s.save_now().unwrap();
        let dir = s.save.as_ref().unwrap().clone();
        let committed = std::fs::read(dir.commit_path()).unwrap();
        dirty(&s, 2, 5);
        let (entered, release) = gate(&mut s, GateAt::Chunk);
        s.request_save().unwrap();
        entered.recv_timeout(Duration::from_secs(20)).unwrap();
        release.send(if panic { Release::Panic } else { Release::Fail }).unwrap();
        assert!(s.wait_for_save().is_err());
        assert!(s.save_error.is_some(), "the player needs the error, not just a log");
        assert_eq!(s.edits().dirty_count(), 2);
        assert_eq!(std::fs::read(dir.commit_path()).unwrap(), committed);
        assert_eq!(dir.read_chunks().unwrap().len(), 1);
        s.save_now().unwrap();
        assert!(s.save_error.is_none());
        assert_eq!(s.edits().dirty_count(), 0);
        assert_eq!(dir.read_commit().unwrap().unwrap().generation, 2);
    }
}

#[test]
fn s1_published_is_not_saved_until_the_final_flush_succeeds() {
    let mut s = fresh("durability");
    dirty(&s, 1, 4);
    s.save_now().unwrap();
    let saved = (s.last_save, s.saved_chunks);
    dirty(&s, 1, 5);
    let (entered, release) = gate(&mut s, GateAt::Published);
    s.request_save().unwrap();
    entered.recv_timeout(Duration::from_secs(20)).unwrap();
    assert_eq!(s.save.as_ref().unwrap().read_commit().unwrap().unwrap().generation, 2);
    assert!(s.poll_save().is_none());
    assert_eq!((s.last_save, s.saved_chunks), saved);
    assert_eq!(s.edits().dirty_count(), 1);
    release.send(Release::Fail).unwrap();
    assert!(s.wait_for_save().is_err());
    assert_eq!((s.last_save, s.saved_chunks), saved);
    assert_eq!(s.edits().dirty_count(), 1);
    assert!(s.save.as_ref().unwrap().path.join("sim.g1.bin").exists());
    s.save_now().unwrap();
    assert_eq!(s.edits().dirty_count(), 0);
}

#[test]
fn s1_snapshot_budget_rejects_before_copying_and_keeps_dirt() {
    let mut s = fresh("budget");
    dirty(&s, 4, 4);
    assert!(s.edits().save_snapshot(1).is_err());
    assert!(SaveSnapshot::capture(&s.engine, &s.bake_sha256, &s.sim, 1).is_err());
    assert_eq!(s.edits().dirty_count(), 4);
    assert!(!s.save.as_ref().unwrap().path.exists());
    let copy = SaveSnapshot::capture(&s.engine, &s.bake_sha256, &s.sim, save::SNAPSHOT_LIMIT).unwrap();
    assert!(copy.bytes < 2 * 1024 * 1024, "four packed chunks plus metadata, not dense world copies");
    drop(copy);
    s.save_now().unwrap();
}

#[test]
fn s1_snapshot_budget_includes_simulation_queues() {
    let mut s = fresh("queue-budget");
    for i in 0..150_000 {
        s.sim.schedule(BlockPos { face: 2, i: i % 500, j: i / 500, k: 547 }, 1);
    }
    assert!(SaveSnapshot::capture(&s.engine, &s.bake_sha256, &s.sim, 2 * 1024 * 1024).is_err(),
        "the queue alone is over two MiB; an empty chunk set must not bypass the limit");
    assert_eq!(s.sim.queued(), 150_000);
    assert!(s.sim.is_dirty());
    assert!(!s.save.as_ref().unwrap().path.exists());
}

#[test]
fn s1_snapshot_owns_water_and_the_registry_identity() {
    let mut s = fresh("water-registry");
    let mut engine = s.engine.with_edits(s.edits().clone());
    engine.registry = Arc::new(crate::world::Registry::from_defs(engine.registry.defs()).unwrap());
    s.engine = Arc::new(engine.with_edits(engine.edits.clone()));
    let key = ChunkKey::neisor(2, 0, 0, 0, 17);
    let mut ids = vec![1; VOXELS];
    ids[0] = s.engine.registry.id("water").unwrap();
    let mut water = vec![0; VOXELS];
    water[0] = 37;
    s.edits().insert_chunk(key, ids.clone(), Some(water.clone()), true);
    let snapshot = SaveSnapshot::capture(&s.engine, &s.bake_sha256, &s.sim, save::SNAPSHOT_LIMIT).unwrap();
    let mut def = s.engine.registry.def(1);
    def.id = s.engine.registry.len() as u16;
    def.name = "s1_new_block".into();
    def.identity = None;
    def.was.clear();
    let added = s.engine.registry.add(def).unwrap();
    water[0] = 145;
    ids[1] = added;
    s.edits().insert_chunk(key, ids, Some(water), true);
    // Submit the already-captured moment only after the live objects changed.
    let dir = s.save.as_ref().unwrap().clone();
    s.save_worker.start(dir.clone(), snapshot, s.edits().clone()).unwrap();
    s.wait_for_save().unwrap();
    let header = dir.read_header().unwrap().unwrap();
    assert!(!header.blocks.iter().any(|(id, _)| *id == added));
    let back = EditStore::new();
    dir.load_into(&s.engine.config, &s.engine.registry, &s.bake_sha256, &back).unwrap();
    assert_eq!(back.water_level(key, 0, 0, 0), Some(37));
    assert_eq!(s.edits().dirty_count(), 1);
    s.save_now().unwrap();
    dir.load_into(&s.engine.config, &s.engine.registry, &s.bake_sha256, &back).unwrap();
    assert_eq!(back.water_level(key, 0, 0, 0), Some(145));
    assert!(dir.read_header().unwrap().unwrap().blocks.iter().any(|(id, _)| *id == added));
}

#[test]
fn s1_quit_displays_waits_and_saves_the_last_edits() {
    let mut s = fresh("quit");
    dirty(&s, 1, 4);
    let (entered, release) = gate(&mut s, GateAt::Chunk);
    s.request_save().unwrap();
    entered.recv_timeout(Duration::from_secs(20)).unwrap();
    dirty(&s, 2, 5);
    let mut quit = SaveOnExit::default();
    assert!(!quit.poll(&mut s).unwrap(), "show a waiting frame first");
    quit.presented();
    assert!(!quit.poll(&mut s).unwrap(), "in-flight write still blocked");
    release.send(Release::Write).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !quit.poll(&mut s).unwrap() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(s.edits().dirty_count(), 0);
    let dir = s.save.as_ref().unwrap();
    assert_eq!(dir.read_commit().unwrap().unwrap().generation, 2);
    let back = EditStore::new();
    dir.load_into(&s.engine.config, &s.engine.registry, &s.bake_sha256, &back).unwrap();
    assert_eq!(back.len(), 2);
    assert_eq!(back.get(ChunkKey::neisor(2, 0, 0, 0, 17), 1, 2, 3), Some(5));
}

#[test]
fn s1_quit_failure_keeps_the_session_open_for_retry() {
    let mut s = fresh("quit-fail");
    dirty(&s, 1, 4);
    let (entered, release) = gate(&mut s, GateAt::Chunk);
    let mut quit = SaveOnExit::default();
    quit.presented();
    assert!(!quit.poll(&mut s).unwrap());
    entered.recv_timeout(Duration::from_secs(20)).unwrap();
    release.send(Release::Fail).unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match quit.poll(&mut s) {
            Err(_) => break,
            Ok(true) => panic!("quit succeeded after a failed final save"),
            Ok(false) => { assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(2)); }
        }
    }
    assert!(s.save_error.is_some());
    assert_eq!(s.edits().dirty_count(), 1);
    s.save_now().unwrap();
    assert!(s.save_error.is_none());
}

#[test]
fn s1_barriers_finish_old_and_latest_generations_before_continuing() {
    for action in ["save", "reload", "record", "preview"] {
        let mut s = fresh(action);
        dirty(&s, 1, 4);
        let (entered, release) = gate(&mut s, GateAt::Chunk);
        s.request_save().unwrap();
        entered.recv_timeout(Duration::from_secs(20)).unwrap();
        dirty(&s, 2, 5);
        let dir = s.save.as_ref().unwrap().clone();
        assert!(s.has_unsaved_changes());
        // An owned helper releases the writer after the barrier is entered.
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            release.send(Release::Write).unwrap();
        });
        match action {
            "record" => { s.start_recording(&dir.path.with_extension("replay"), "barrier").unwrap(); s.stop_recording().unwrap(); }
            "reload" => { s.reload().unwrap(); }
            _ => { s.save_now().unwrap(); }
        }
        releaser.join().unwrap();
        assert!(!s.save_in_flight());
        assert_eq!(s.edits().dirty_count(), 0);
        assert_eq!(dir.read_commit().unwrap().unwrap().generation, 2);
        assert_eq!(dir.read_chunks().unwrap().len(), 2);
        if action == "reload" {
            assert!(!s.has_unsaved_changes(), "the loaded generation is already saved");
        }
        if action == "preview" {
            assert!(!s.has_unsaved_changes(), "an unchanged preview must not touch the save again");
            s.save = None;
            assert!(!s.request_save().unwrap());
        }
    }
}

#[test]
fn s1_replacing_a_chunk_during_a_write_keeps_its_new_revision_dirty() {
    let store = EditStore::new();
    let key = ChunkKey::neisor(2, 0, 0, 0, 17);
    store.insert_chunk(key, vec![4; VOXELS], None, true);
    let snapshot = store.dirty_snapshot();
    store.insert_chunk(key, vec![5; VOXELS], None, true);
    store.acknowledge(&[(key, snapshot[0].rev)]);
    assert_eq!(store.dirty_count(), 1, "a replacement must not reuse a saved revision");
}

#[test]
fn s1_failed_simulation_only_save_is_retried_by_autosave() {
    let mut s = fresh("sim-only");
    s.sim.set_radius(22.0);
    let (entered, release) = gate(&mut s, GateAt::Published);
    s.request_save().unwrap();
    entered.recv_timeout(Duration::from_secs(20)).unwrap();
    release.send(Release::Fail).unwrap();
    assert!(s.wait_for_save().is_err());
    assert_eq!(s.edits().dirty_count(), 0);
    s.save_if_due(Instant::now() + Duration::from_secs(3)).unwrap();
    assert!(s.save_in_flight(), "retry even without a dirty chunk to trigger it");
    s.wait_for_save().unwrap();
    assert!(s.save_error.is_none());
    assert!(s.last_save.is_some());
}

/// The baseline measurement leaves a real pre-S1 save intact for this check.
#[test]
#[ignore]
fn s1_load_preserved_main_save() {
    let path = std::env::var_os("S1_MAIN_SAVE").expect("set S1_MAIN_SAVE to the baseline's cost-400 directory");
    let s = crate::replay::tests::fresh(None);
    let dir = SaveDir::new(path);
    let store = EditStore::new();
    let report = dir.load_into(&s.engine.config, &s.engine.registry, &s.bake_sha256, &store).unwrap();
    assert_eq!(report.chunks, 400);
    assert_eq!(store.get(ChunkKey::neisor(2, 0, 0, 0, 17), 1, 2, 3), Some(6));
    Session::load_sim(Some(&dir), s.sim.config, &s.engine.registry).unwrap();
    println!("Pre-S1 save loaded read-only: {} chunks, generation {} from {}", report.chunks, report.generation, dir.path.display());
}

#[test]
fn s1_autosaves_do_not_change_fixed_steps_or_replay() {
    let mut a = fresh("determinism");
    let mut b = crate::replay::tests::fresh(None);
    for s in [&mut a, &mut b] {
        s.sim.set_radius(35.0);
        s.fill_rel((-2, -2, 0), (2, 2, 8), s.engine.registry.id("air").unwrap(), None);
        s.fill_rel((-2, -2, -1), (2, 2, -1), s.engine.registry.id("stone").unwrap(), None);
        s.fill_rel((0, 0, 4), (0, 0, 7), s.engine.registry.id("sand").unwrap(), None);
    }
    let dir = a.save.as_ref().unwrap().path.clone();
    let path = dir.with_extension("replay");
    a.start_recording(&path, "autosave timing is not input").unwrap();
    // Hold the sim-only generation's final flush while simulation keeps stepping.
    let (entered, release) = gate(&mut a, GateAt::Published);
    a.request_save().unwrap();
    entered.recv_timeout(Duration::from_secs(20)).unwrap();
    for _ in 0..120 {
        a.step(&Input::default(), player::STEP);
        b.step(&Input::default(), player::STEP);
        assert_eq!(a.sim.state_digest(&a.engine.world), b.sim.state_digest(&b.engine.world));
        assert_eq!(a.player.pos, b.player.pos);
        assert!(!a.request_save().unwrap());
    }
    release.send(Release::Write).unwrap();
    a.save_now().unwrap();
    a.stop_recording().unwrap();
    assert_eq!(a.sim_hash(), b.sim_hash());
    let rep = crate::replay::read(&path).unwrap();
    let scratch = dir.join("_replay/verify");
    let mut back = crate::replay::session_from(&a, &rep, &scratch).unwrap();
    let report = crate::replay::Runner::new(&rep, &back).run(&mut back);
    assert!(report.ok(), "{}", report.text());
}

struct KillFs { root: PathBuf, stage: String }
impl SaveFs for KillFs {
    fn write_synced(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        let stop = if self.stage == "chunk" {
            path.extension().is_some_and(|e| e == "chunk")
        } else { path.file_name().is_some_and(|e| e == "commit.tmp") };
        if stop {
            // A real partial file, then the process is forcibly killed by its parent.
            OsFs.write_synced(path, &bytes[..bytes.len() / 2])?;
            std::fs::write(self.root.join("kill-ready"), b"partial generation on disk")?;
            loop { std::thread::park_timeout(Duration::from_secs(1)); }
        }
        OsFs.write_synced(path, bytes)
    }
    fn replace_durably(&self, from: &Path, to: &Path) -> std::io::Result<()> { OsFs.replace_durably(from, to) }
    fn sync_dir(&self, dir: &Path) -> std::io::Result<()> { OsFs.sync_dir(dir) }
    fn remove(&self, path: &Path) -> std::io::Result<()> { OsFs.remove(path) }
}

#[test]
fn s1_kill_child() {
    let Some(root) = std::env::var_os("S1_KILL_SAVE") else { return };
    let root = PathBuf::from(root);
    let mut s = fresh("kill-child");
    s.save = Some(SaveDir::new(&root));
    s.save.as_ref().unwrap().load_into(&s.engine.config, &s.engine.registry, &s.bake_sha256, s.edits()).unwrap();
    dirty(&s, 2, 5);
    s.save_worker = SaveWorker::with_fs(Arc::new(KillFs { root, stage: std::env::var("S1_KILL_STAGE").unwrap() }));
    s.request_save().unwrap();
    s.wait_for_save().unwrap();
    panic!("parent should kill this process during the write");
}

#[test]
fn s1_kill_mid_write_keeps_the_previous_generation_loadable() {
    struct OwnedChild(std::process::Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
    }
    for stage in ["chunk", "commit"] {
        let mut s = fresh(&format!("kill-{stage}"));
        dirty(&s, 2, 4);
        s.save_now().unwrap();
        let dir = s.save.as_ref().unwrap().clone();
        let commit = std::fs::read(dir.commit_path()).unwrap();
        let sim = dir.read_sim().unwrap();
        let mut child = OwnedChild(std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "session::save_tests::s1_kill_child", "--test-threads=1"])
            .env("S1_KILL_SAVE", &dir.path).env("S1_KILL_STAGE", stage)
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(45);
        while !dir.path.join("kill-ready").exists() {
            assert!(child.0.try_wait().unwrap().is_none(), "child exited before the injected kill point");
            assert!(Instant::now() < deadline, "child never reached the kill point");
            std::thread::sleep(Duration::from_millis(10));
        }
        child.0.kill().unwrap();
        assert!(!child.0.wait().unwrap().success());
        assert_eq!(std::fs::read(dir.commit_path()).unwrap(), commit);
        assert_eq!(dir.read_sim().unwrap(), sim);
        let back = EditStore::new();
        assert_eq!(dir.load_into(&s.engine.config, &s.engine.registry, &s.bake_sha256, &back).unwrap().generation, 1);
        assert_eq!(back.get(ChunkKey::neisor(2, 0, 0, 0, 17), 1, 2, 3), Some(4));
        dirty(&s, 2, 6);
        s.save_now().unwrap();
        assert_eq!(dir.read_commit().unwrap().unwrap().generation, 2);
        assert_eq!(dir.read_chunks().unwrap().len(), 2);
    }
}

/// Run before and after S1 with identical dirty chunks and actual durable I/O.
#[test]
#[ignore]
fn s1_main_thread_save_cost() {
    for n in [400, 2000] {
        let mut s = fresh(&format!("cost-{n}"));
        for pass in 0..3 {
            dirty(&s, n, 4 + pass);
            assert_eq!(s.edits().dirty_count(), n as usize);
            let start = Instant::now();
            s.save_if_due(Instant::now() + Duration::from_secs(20)).unwrap();
            let request_ms = start.elapsed().as_secs_f64() * 1000.0;
            s.save_now().unwrap();
            println!("PROVISIONAL S1 {n} dirty chunks pass {pass}: main {request_ms:.3} ms, through barrier {:.3} ms", start.elapsed().as_secs_f64() * 1000.0);
        }
    }
}
