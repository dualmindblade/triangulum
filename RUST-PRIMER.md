# Rust, using Triangulum as the textbook

Written for Austin: knows C# and TypeScript well, has Haskell-derived
FP basics, wants straight explanations. Every example here is real
code from this repo - open the files and read around them.

## 1. Philosophy

Rust's founding bet: the two hardest bug classes in systems code -
memory errors and data races - can be eliminated AT COMPILE TIME by
tracking, in the type system, which code OWNS each value and which
code is merely BORROWING it. That tracking is strict enough that the
compiler can prove safety, which means:

- No garbage collector. Memory is freed at deterministic points the
  compiler derives from ownership. For a game this is the headline:
  a GC pause IS a frame hitch, and you have spent three days feeling
  exactly what frame hitches do to this game. Rust's promise is that
  the language itself never injects one.
- No null, no exceptions. Absence and failure are ordinary values
  (`Option<T>`, `Result<T, E>`) that the type system forces you to
  handle. C#'s `NullReferenceException` does not exist here as a
  runtime concept - the equivalent mistake fails to compile.
- Zero-cost abstraction (inherited from C++'s culture): iterators,
  generics, and traits compile down to the same machine code you
  would write by hand. You pay for what you use, at compile time.
- Explicitness: allocation, copying, and mutation are visible in the
  source. When `weather.rs` clones a struct you can see the `.clone()`;
  when it passes by reference you see the `&`. Nothing large is copied
  silently.

The cost of all this is the famous learning curve: the compiler
rejects programs a C# programmer considers obviously fine, because it
cannot PROVE them fine. The skill of writing Rust is largely the skill
of structuring code so ownership is provable. It front-loads pain:
programs that compile tend to be free of whole bug categories, which
is why our "one truth, two renderers" contract - byte-identical
deterministic output across threads - is realistic here at all.

## 2. Use cases - where Rust is the right tool

- Game engines and simulation (us): predictable frame times, tight
  memory control, safe parallelism (`rayon` fans terrain builds across
  cores; the compiler proves workers cannot race on shared state).
- Network services: our multiplayer server (viewer/server) runs on
  `tokio`, an async runtime. Rust and Go compete here; Rust wins when
  latency variance matters.
- CLI tools, WASM, embedded, OS components - anywhere C/C++ lived.
- NOT the fastest path for: quick scripts, exploratory prototypes,
  GC-friendly business logic. We keep Python for the bake pipeline
  and the reels for exactly this reason - right tool per layer.

## 3. Coming from your languages

From C#:
- `struct` is the default and it lives wherever you declare it (stack,
  inside a Vec, inside another struct) - not "value type vs reference
  type" as two class-like worlds. There is no class. `Box<T>` puts a
  value on the heap explicitly.
- No inheritance at all. Composition + traits (below) replace it.
- Immutability is the default: `let x = 5;` is final; mutation
  requires `let mut x`. In C# terms, everything is `readonly` unless
  marked otherwise - inverted from C#.
- No exceptions for recoverable errors; `Result` + the `?` operator
  replace try/catch (panics exist but mean "bug", not "error"; the
  exit code 101 you saw was a panic).
- Generics are compiled by monomorphization (like C++ templates, or
  C# generics over value types): each concrete instantiation gets its
  own optimized machine code.

From TypeScript:
- Rust enums are tagged unions like TS discriminated unions, but
  first-class and exhaustively checked. Where TS has
  `{kind: "tile"} | {kind: "chunk"}`, renderer.rs has
  `enum DrawKey { Tile(TileKey), Chunk(ChunkKey) }` - each variant
  CARRIES its payload, and `match` will not compile if you forget one.
- Types are nominal, not structural, and they exist at runtime (a
  `u32` is 4 bytes, period). There is no `any` escape hatch.

From Haskell (more relevant than you may expect):
- `Option<T>` IS `Maybe`; `Result<T, E>` IS `Either E T`. The `?`
  operator is a specialized early-return `do`-notation for them.
- Traits are typeclasses, almost exactly: `impl Display for Camera`
  is `instance Show Camera`. Deriving works the same way:
  `#[derive(Clone, Debug, serde::Deserialize)]` on `WeatherTuning`
  (weather.rs) auto-writes those instances.
- Pattern matching, algebraic data types, expression-orientation
  (blocks and `if` return values - the last expression, no `return`
  needed) all carry over.
- The big differences: strict evaluation (no laziness), no HKTs (no
  monad abstraction - each type implements its own combinators), and
  mutation is embraced rather than exiled, but CONTROLLED by ownership.

## 4. The lesson: ownership, moves, and borrows

This is the genuinely new concept, so it gets the space.

### 4.1 Every value has exactly one owner

Assignment and argument-passing MOVE ownership by default (for types
that are not trivially copyable). After a move, the source variable is
dead - using it is a compile error.

Real example, viewer/src/main.rs (the capture path):

    let (device, queue) = pollster::block_on(adapter.request_device(...))?;
    let mut renderer = Renderer::new(device, queue, ...);
    // `device` and `queue` are GONE from this scope now. They moved
    // into Renderer::new, which moved them into the Renderer struct.
    // Any later use of `device` here fails to compile.

In C#, `renderer` and this function would both hold references to the
same device object, and its lifetime would be "whenever the GC decides
after nobody uses it". In Rust, Renderer owns them; when the Renderer
is dropped, they are freed, deterministically, then.

Small types opt out of moving via `Copy` (integers, floats, our
`Camera` - see `#[derive(Clone, Copy, ...)]` in camera.rs). Copy types
are duplicated bit-for-bit on assignment, C#-struct-style. `Clone` is
the explicit, possibly-expensive duplicate: you must write `.clone()`.

### 4.2 Borrowing: use without owning

`&T` is a shared (read-only) borrow; `&mut T` is an exclusive
(read-write) borrow. THE rule, the one that everything else follows
from:

    At any moment a value has EITHER any number of `&T` borrows
    OR exactly one `&mut T` borrow. Never both.

Real example, viewer/src/terrain.rs:

    pub fn build_tile_at_season(
        planet: &Planet,          // shared borrow: read the world
        key: TileKey,             // Copy type: passed by value
        exaggeration: f64,
        season: StructuralSeason,
    ) -> TileMesh                 // returns an OWNED mesh to the caller

Sixteen rayon workers call this concurrently with the same `&Planet`.
That is safe, and the compiler PROVES it is safe, because `&Planet`
grants no mutation. If any worker tried to mutate the planet through
that reference, compilation fails. This is the data-race story: races
require aliasing + mutation, and the borrow rule makes that
combination unrepresentable.

Contrast renderer.rs:

    pub fn draw(&mut self, target: &wgpu::TextureView, ...) -> usize

`&mut self` means: while draw() runs, NOTHING else can even read the
renderer. One writer, zero readers, enforced. In C# you would document
"not thread-safe" and hope; here it is a type.

### 4.3 Sharing across threads: Arc

When two threads genuinely need the same data and neither is the
clear owner, you opt into shared ownership explicitly with
`Arc<T>` (atomically reference-counted pointer). Real example from
the prefetch code in renderer.rs:

    let tx = self.tile_tx.clone();
    let planet = Arc::clone(planet);      // bump refcount: cheap
    let season = structural_season;       // Copy
    rayon::spawn(move || {
        let mesh = build_tile_at_season(&planet, key, exagg, season);
        let _ = tx.send((key, epoch, season.bucket, mesh));
    });

Three things to notice:

- `Arc::clone` copies a POINTER and increments a counter - it does not
  copy the planet. The last Arc dropped frees it.
- `move ||` is a closure that takes OWNERSHIP of everything it
  captures (the Arc, the sender, the key). It must: the closure will
  run on another thread, possibly after this function returns, so
  borrowing from the current stack frame would be unsound - and the
  compiler rejects exactly that if you forget `move`.
- Results come back over a CHANNEL (`tx.send`), not shared mutable
  state. The draw loop drains the receiver later. This "share by
  communicating" pattern is everywhere in the renderer (tile_tx/rx,
  chunk_tx/rx) and in the multiplayer server.

`Arc<T>` alone is immutable sharing. Shared MUTABLE state needs an
explicit lock wrapper: the server (viewer/server/src/main.rs) uses
`Mutex<JournalStore>` inside an Arc'd ServerState, and the type system
makes it impossible to touch the journal without locking - there is no
"forgot to take the lock" bug; the data is physically inside the lock.

### 4.4 Lifetimes, in one paragraph

A borrow must never outlive what it borrows. Usually the compiler
infers this invisibly. When a signature is ambiguous you annotate
relationships with names like `<'a>` - read `&'a str` as "a reference
valid for at least region 'a". You will rarely write one for months;
when the compiler asks, it is telling you it cannot prove your
reference dies before its target does, and nine times out of ten the
right fix is restructuring (return owned data, or clone) rather than
annotation. Our codebase, ~20k lines, contains almost none - by design.

### 4.5 Option, Result, and `?`

viewer/multiplayer/src/journal.rs is the cleanest file to read for
this. Signature:

    pub fn open(path: impl Into<PathBuf>) -> Result<Self, JournalError>

and inside, lines like:

    let mut bytes = Vec::new();
    std::fs::File::open(&path)?.read_to_end(&mut bytes)?;

Each `?` means: if this `Result` is `Err`, return it from THIS
function right now (converting the error type if a conversion is
defined); if `Ok`, unwrap and continue. It is try/catch flattened
into the type system - the function's signature tells you it can
fail, the call sites are marked, and ignoring a `Result` is a
compiler warning. `Option<T>` works the same for absence:
`self.gpu_timers.as_ref()?` in renderer.rs returns None early if
timers are off.

`unwrap()`/`expect("msg")` convert Err/None into a PANIC - crash with
a backtrace. House rule visible in the code: fine in tests, fine for
"impossible by construction" cases, avoided on any path a user can
reach. (Exit code 101, which you met yesterday, is what a panic looks
like from outside.)

### 4.6 Enums and match are the workhorse

viewer/src/camera.rs:

    pub enum CameraMode {
        Focused(crate::orbits::BodyId),   // variant WITH data
        Freecam,
    }

    match self.mode {
        CameraMode::Focused(body) => ...,  // `body` bound right here
        CameraMode::Freecam => ...,
    }

Add a third mode someday and every non-exhaustive `match` in the
project becomes a compile error listing exactly what to update. This
is the mechanism behind half our refactor safety. The multiplayer
protocol (multiplayer/src/protocol.rs) is one big enum of messages
for the same reason: the server's match statement provably handles
every message kind.

## 5. Reading and poking at this codebase

Setup: install rustup (rustup.rs), then in VS Code add the
rust-analyzer extension - inline types, jump-to-definition, and it
runs the borrow checker as you type. The project builds with
`cargo build --release` from viewer/ (you know this part).

Reading order, easy to deep:

1. viewer/src/camera.rs - small, self-contained, pure math, every
   concept from section 4 appears.
2. viewer/multiplayer/src/journal.rs - clean I/O, Result/? throughout,
   unit tests at the bottom in the standard `#[cfg(test)]` pattern.
3. viewer/src/orbits.rs - pure functions of time, the D-5 clock.
4. viewer/src/weather.rs - bigger, but it is all pure functions and
   you know what every one of them does from the outside already.
5. viewer/src/renderer.rs + terrain.rs - the deep end: threading,
   channels, GPU, caches.

First exercises, in order of ambition (all safely gauntleted - run
`cargo test --release --lib` and the suites after each):

1. Change a weather_tuning.json default in code (weather.rs
   `impl Default for WeatherTuning`) and trace where the value flows.
2. Add a unit test to journal.rs asserting something you believe
   (e.g. sequence numbers survive a reopen). `cargo test -p
   triangulum-multiplayer` runs just that crate.
3. Add an operator console command to the server - `uptime`, say.
   Copy the `["players"]` match arm in server/src/main.rs; the
   compiler will walk you through everything you miss.
4. Add a `probe`-style field to the play harness sidecar
   (examples/play.rs) - touches the lib/binary boundary.

The compiler is the tutor: `cargo build` errors in Rust are unusually
good, and `rustc --explain E0502` (any error code) gives a worked
example of the exact rule you tripped. When it rejects something you
think is fine, the interesting question is never "how do I silence
it" but "what aliasing does it see that I do not".
