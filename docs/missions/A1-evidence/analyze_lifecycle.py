#!/usr/bin/env python3
"""Partition B-22 tile lifecycles for the fixed ice-session h3 hold.

The cohort is the union of on-screen missing keys reported by the 1 Hz
summaries at the final h3 pose.  Each key is followed from its latest
`missing` event before it entered that cohort through `first_exact_draw`.
The state machine makes the requested partition exhaustive:

    unadmitted -> queue -> build -> land/swap -> exact draw

An eviction returns the key to `unadmitted`.  Output times use the trace's
single monotonic `t_us` origin; they are wall time, not simulated flight time.
"""

from __future__ import annotations

import argparse
import csv
import json
import math
import statistics
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Iterable


H3_LATITUDE_DEG = 41.518883
H3_LONGITUDE_DEG = 122.511318
H3_ALTITUDE_KM = 3.0498
H3_YAW_DEG = 92.98
H3_PITCH_DEG = -68.3

HUNT3_KEYS = [
    f"f4L11({ix},{iy})"
    for iy in (1652, 1653)
    for ix in range(2002, 2006)
]

PHASES = ("unadmitted", "queue", "build", "land_swap")


def close(a: Any, b: float, tolerance: float = 0.001) -> bool:
    return isinstance(a, (int, float)) and abs(float(a) - b) <= tolerance


def is_h3_summary(event: dict[str, Any]) -> bool:
    return (
        event.get("event") == "summary"
        and close(event.get("latitude_deg"), H3_LATITUDE_DEG)
        and close(event.get("longitude_deg"), H3_LONGITUDE_DEG)
        and close(event.get("altitude_km"), H3_ALTITUDE_KM)
        and close(event.get("yaw_deg"), H3_YAW_DEG, 0.01)
        and close(event.get("pitch_deg"), H3_PITCH_DEG, 0.01)
    )


def percentile(values: Iterable[float], percent: float) -> float | None:
    """Linear percentile, matching NumPy's default interpolation."""
    ordered = sorted(values)
    if not ordered:
        return None
    position = (len(ordered) - 1) * percent / 100.0
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return ordered[lower]
    fraction = position - lower
    return ordered[lower] * (1.0 - fraction) + ordered[upper] * fraction


def distribution(values: Iterable[float]) -> dict[str, float | int | None]:
    materialized = list(values)
    return {
        "count": len(materialized),
        "sum": sum(materialized),
        "mean": statistics.fmean(materialized) if materialized else None,
        "median": statistics.median(materialized) if materialized else None,
        "p95": percentile(materialized, 95.0),
        "max": max(materialized) if materialized else None,
    }


def rounded(value: Any) -> Any:
    if isinstance(value, float):
        return round(value, 6)
    if isinstance(value, dict):
        return {key: rounded(item) for key, item in value.items()}
    if isinstance(value, list):
        return [rounded(item) for item in value]
    return value


def load_trace(path: Path) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    with path.open("r", encoding="utf-8") as source:
        for line_number, line in enumerate(source, 1):
            try:
                event = json.loads(line)
            except json.JSONDecodeError as error:
                raise SystemExit(
                    f"{path}:{line_number}: malformed lifecycle JSON: {error}"
                ) from error
            if event.get("stream_lifecycle") == 1:
                events.append(event)
    if not events:
        raise SystemExit(f"{path}: no stream_lifecycle=1 events")
    return events


def transition(
    durations_us: dict[str, int],
    current_phase: str,
    current_time_us: int,
    next_phase: str,
    next_time_us: int,
) -> tuple[str, int]:
    if next_time_us < current_time_us:
        raise ValueError("lifecycle event time moved backwards")
    durations_us[current_phase] += next_time_us - current_time_us
    return next_phase, next_time_us


def partition_key(
    key: str,
    observation: dict[str, Any],
    events_by_key: dict[str, list[dict[str, Any]]],
    h3_end_us: int,
) -> dict[str, Any]:
    key_events = events_by_key[key]
    observed_us = int(observation["observed_t_us"])
    missing_candidates = [
        event
        for event in key_events
        if event.get("event") == "missing" and int(event["t_us"]) <= observed_us
    ]
    if not missing_candidates:
        # A key may enter the fixed-pose cohort between summary ticks.
        missing_candidates = [
            event
            for event in key_events
            if event.get("event") == "missing"
            and observed_us <= int(event["t_us"]) <= h3_end_us
        ]
    if not missing_candidates:
        return {
            **observation,
            "key": key,
            "status": "missing_event_absent",
        }

    missing = (
        missing_candidates[-1]
        if int(missing_candidates[-1]["t_us"]) <= observed_us
        else missing_candidates[0]
    )
    start_us = int(missing["t_us"])
    draws = [
        event
        for event in key_events
        if event.get("event") == "first_exact_draw"
        and int(event["t_us"]) >= max(start_us, observed_us)
    ]
    draw = draws[0] if draws else None
    finish_us = int(draw["t_us"]) if draw else h3_end_us

    durations_us = {phase: 0 for phase in PHASES}
    phase = "unadmitted"
    phase_started_us = start_us
    sources: list[str] = []
    worker_attempts = 0
    worker_done_count = 0
    reported_build_ms = 0.0
    accepted_receives = 0
    rejected_receives = 0
    eviction_reasons: Counter[str] = Counter()
    state_anomalies: list[str] = []

    for event in key_events:
        event_us = int(event["t_us"])
        if event_us < start_us or event_us > finish_us:
            continue
        event_name = event.get("event")
        if event_name == "missing" and event is not missing:
            phase, phase_started_us = transition(
                durations_us, phase, phase_started_us, "unadmitted", event_us
            )
        elif event_name == "admit":
            phase, phase_started_us = transition(
                durations_us, phase, phase_started_us, "queue", event_us
            )
            sources.append(str(event.get("source", "unknown")))
        elif event_name == "worker_start":
            if phase != "queue":
                state_anomalies.append(f"worker_start_from_{phase}")
            phase, phase_started_us = transition(
                durations_us, phase, phase_started_us, "build", event_us
            )
            worker_attempts += 1
        elif event_name == "worker_done":
            if phase != "build":
                state_anomalies.append(f"worker_done_from_{phase}")
            phase, phase_started_us = transition(
                durations_us, phase, phase_started_us, "land_swap", event_us
            )
            worker_done_count += 1
            reported_build_ms += float(event.get("build_ms", 0.0))
        elif event_name == "received":
            if (
                event.get("pending_match")
                and event.get("season_match")
                and not event.get("moving_drop")
            ):
                accepted_receives += 1
            else:
                rejected_receives += 1
        elif event_name == "evict":
            phase, phase_started_us = transition(
                durations_us, phase, phase_started_us, "unadmitted", event_us
            )
            eviction_reasons[str(event.get("reason", "unknown"))] += 1
        elif event_name == "first_exact_draw" and event is draw:
            durations_us[phase] += event_us - phase_started_us
            phase_started_us = event_us
            break
    else:
        durations_us[phase] += finish_us - phase_started_us

    total_us = finish_us - start_us
    partition_us = sum(durations_us.values())
    if partition_us != total_us:
        state_anomalies.append(f"partition_delta_us={partition_us - total_us}")

    durations_ms = {
        f"t_{phase}_ms": durations_us[phase] / 1000.0 for phase in PHASES
    }
    dominant_phase = max(PHASES, key=lambda item: durations_us[item])
    return {
        **observation,
        "key": key,
        "status": "complete" if draw else "censored_at_h3_end",
        "missing_episode": missing.get("episode"),
        "missing_frame": missing.get("frame"),
        "missing_t_us": start_us,
        "draw_frame": draw.get("frame") if draw else None,
        "draw_t_us": int(draw["t_us"]) if draw else None,
        "t_total_ms": total_us / 1000.0,
        **durations_ms,
        "dominant_phase": dominant_phase,
        "admit_sources": "+".join(sources),
        "worker_attempts": worker_attempts,
        "worker_done_count": worker_done_count,
        "reported_build_ms": reported_build_ms,
        "accepted_receives": accepted_receives,
        "rejected_receives": rejected_receives,
        "eviction_count": sum(eviction_reasons.values()),
        "eviction_reasons": "+".join(
            f"{reason}:{count}" for reason, count in sorted(eviction_reasons.items())
        ),
        "state_anomalies": "+".join(state_anomalies),
    }


def write_csv(path: Path, rows: list[dict[str, Any]], fields: list[str]) -> None:
    with path.open("w", newline="", encoding="utf-8") as destination:
        writer = csv.DictWriter(destination, fieldnames=fields, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)


def analyze(trace_path: Path, output_prefix: Path) -> None:
    events = load_trace(trace_path)
    h3_summaries = [event for event in events if is_h3_summary(event)]
    if not h3_summaries:
        raise SystemExit(f"{trace_path}: no exact h3-pose summaries")
    h3_summaries.sort(key=lambda event: int(event["t_us"]))
    h3_start = h3_summaries[0]
    h3_end = h3_summaries[-1]
    h3_start_us = int(h3_start["t_us"])
    h3_end_us = int(h3_end["t_us"])

    events_by_key: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for event in events:
        if "key" in event:
            events_by_key[str(event["key"])].append(event)
    for key_events in events_by_key.values():
        key_events.sort(key=lambda event: (int(event["t_us"]), int(event["seq"])))

    cohort: dict[str, dict[str, Any]] = {}
    for summary in h3_summaries:
        for item in summary.get("on_screen_missing_keys", []):
            key = str(item["key"])
            cohort.setdefault(
                key,
                {
                    "observed_frame": summary["frame"],
                    "observed_t_us": summary["t_us"],
                    "projected_px_area": item.get("projected_px_area"),
                    "gap_rank": item.get("gap_rank"),
                    "face": item.get("face"),
                    "level": item.get("level"),
                    "ix": item.get("ix"),
                    "iy": item.get("iy"),
                    "deep": item.get("deep"),
                },
            )
    # Cover an on-screen miss that appears and resolves between 1 Hz summaries.
    for event in events:
        if (
            event.get("event") == "missing"
            and event.get("on_screen")
            and h3_start_us <= int(event["t_us"]) <= h3_end_us
        ):
            key = str(event["key"])
            cohort.setdefault(
                key,
                {
                    "observed_frame": event["frame"],
                    "observed_t_us": event["t_us"],
                    "projected_px_area": event.get("projected_px_area"),
                    "gap_rank": event.get("gap_rank"),
                    "face": event.get("face"),
                    "level": event.get("level"),
                    "ix": event.get("ix"),
                    "iy": event.get("iy"),
                    "deep": event.get("deep"),
                },
            )

    rows = [
        partition_key(key, observation, events_by_key, h3_end_us)
        for key, observation in cohort.items()
    ]
    rows.sort(
        key=lambda row: (
            -(float(row.get("projected_px_area") or 0.0)),
            str(row["key"]),
        )
    )
    completed = [row for row in rows if row.get("status") == "complete"]
    censored = [row for row in rows if row.get("status") != "complete"]

    phase_fields = [
        "t_total_ms",
        "t_unadmitted_ms",
        "t_queue_ms",
        "t_build_ms",
        "t_land_swap_ms",
        "reported_build_ms",
    ]
    aggregate = {
        field: distribution(float(row[field]) for row in completed)
        for field in phase_fields
    }
    phase_sums = {
        phase: sum(float(row[f"t_{phase}_ms"]) for row in completed)
        for phase in PHASES
    }
    total_sum = sum(float(row["t_total_ms"]) for row in completed)

    high_impact = [
        row for row in completed if float(row.get("projected_px_area") or 0.0) >= 10_000
    ]
    high_impact_aggregate = {
        field: distribution(float(row[field]) for row in high_impact)
        for field in phase_fields
    }

    trace_evictions = [
        event for event in events if event.get("event") == "evict"
    ]
    h3_evictions = [
        event
        for event in trace_evictions
        if h3_start_us <= int(event["t_us"]) <= h3_end_us
    ]

    first_draw_keys = {
        str(event["key"])
        for event in events
        if event.get("event") == "first_exact_draw"
    }
    h3_accepted_receives = [
        event
        for event in events
        if event.get("event") == "received"
        and h3_start_us <= int(event["t_us"]) <= h3_end_us
        and event.get("pending_match")
        and event.get("season_match")
        and not event.get("moving_drop")
    ]
    h3_received_keys = {str(event["key"]) for event in h3_accepted_receives}
    never_first_exact_keys = sorted(h3_received_keys - first_draw_keys)
    cohort_received_never_exact = sorted(
        {
            row["key"]
            for row in rows
            if int(row.get("accepted_receives") or 0) > 0
            and row.get("status") != "complete"
        }
    )

    blocked_by_key: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for event in events:
        if event.get("event") == "gap_blocked":
            blocked_by_key[str(event["key"])].append(event)

    hunt_rows: list[dict[str, Any]] = []
    for key in HUNT3_KEYS:
        key_events = events_by_key.get(key, [])
        blocks = blocked_by_key.get(key, [])
        admits = [event for event in key_events if event.get("event") == "admit"]
        starts = [
            event for event in key_events if event.get("event") == "worker_start"
        ]
        dones = [
            event for event in key_events if event.get("event") == "worker_done"
        ]
        receives = [
            event
            for event in key_events
            if event.get("event") == "received"
            and event.get("pending_match")
            and event.get("season_match")
            and not event.get("moving_drop")
        ]
        draws = [
            event
            for event in key_events
            if event.get("event") == "first_exact_draw"
        ]
        wanted_count_evictions = [
            event
            for event in key_events
            if event.get("event") == "evict"
            and event.get("reason") == "count1500"
            and event.get("live_wanted")
        ]
        hunt_rows.append(
            {
                "key": key,
                "blocked_reasons": "+".join(
                    sorted({str(event.get("reason")) for event in blocks})
                ),
                "first_block_frame": blocks[0].get("frame") if blocks else None,
                "first_block_rank": blocks[0].get("gap_rank") if blocks else None,
                "first_block_productive": (
                    f"{blocks[0].get('productive_total')}/"
                    f"{blocks[0].get('productive_limit')}"
                    if blocks
                    else ""
                ),
                "first_block_budget": (
                    f"{blocks[0].get('admitted_this_frame')}/"
                    f"{blocks[0].get('budget')}"
                    if blocks
                    else ""
                ),
                "admit_sources": "+".join(
                    str(event.get("source", "unknown")) for event in admits
                ),
                "worker_starts": len(starts),
                "worker_done": len(dones),
                "reported_build_ms": sum(
                    float(event.get("build_ms", 0.0)) for event in dones
                ),
                "accepted_receives": len(receives),
                "wanted_count1500_evictions": len(wanted_count_evictions),
                "wanted_count1500_evict_frames": "+".join(
                    str(event.get("frame")) for event in wanted_count_evictions
                ),
                "first_exact_draw_frame": draws[0].get("frame") if draws else None,
                "first_exact_draw_t_us": draws[0].get("t_us") if draws else None,
            }
        )

    summary = {
        "trace": str(trace_path),
        "clock": "renderer_monotonic_t_us_wall_time",
        "h3": {
            "start_frame": h3_start["frame"],
            "end_frame": h3_end["frame"],
            "start_t_us": h3_start_us,
            "end_t_us": h3_end_us,
            "wall_duration_ms": (h3_end_us - h3_start_us) / 1000.0,
            "summary_count": len(h3_summaries),
            "first_summary": {
                key: h3_start.get(key)
                for key in (
                    "wanted",
                    "uncached",
                    "covered_missing",
                    "on_screen_missing",
                    "gap_pending",
                    "gap_unadmitted",
                    "exact_selected_draws",
                    "productive_workers_active",
                )
            },
            "last_summary": {
                key: h3_end.get(key)
                for key in (
                    "wanted",
                    "uncached",
                    "covered_missing",
                    "on_screen_missing",
                    "gap_pending",
                    "gap_unadmitted",
                    "exact_selected_draws",
                    "productive_workers_active",
                )
            },
        },
        "cohort": {
            "on_screen_missing_keys": len(rows),
            "completed": len(completed),
            "censored_or_missing_event": len(censored),
            "dominant_phase_counts": dict(
                Counter(str(row["dominant_phase"]) for row in completed)
            ),
            "admit_source_counts": dict(
                Counter(
                    source
                    for row in completed
                    for source in str(row["admit_sources"]).split("+")
                    if source
                )
            ),
            "worker_attempts": sum(int(row["worker_attempts"]) for row in completed),
            "evictions": sum(int(row["eviction_count"]) for row in completed),
            "accepted_receives_never_exact": cohort_received_never_exact,
            "partition": aggregate,
            "phase_share_of_total": {
                phase: phase_sums[phase] / total_sum if total_sum else None
                for phase in PHASES
            },
            "high_impact_area_at_least_10000_px": {
                "count": len(high_impact),
                "partition": high_impact_aggregate,
            },
        },
        "evictions": {
            "whole_trace": {
                "total": len(trace_evictions),
                "by_reason_and_wanted": {
                    f"{reason}|live_wanted={str(wanted).lower()}": count
                    for (reason, wanted), count in sorted(
                        Counter(
                            (
                                str(event.get("reason")),
                                bool(event.get("live_wanted")),
                            )
                            for event in trace_evictions
                        ).items()
                    )
                },
            },
            "h3": {
                "total": len(h3_evictions),
                "by_reason_and_wanted": {
                    f"{reason}|live_wanted={str(wanted).lower()}": count
                    for (reason, wanted), count in sorted(
                        Counter(
                            (
                                str(event.get("reason")),
                                bool(event.get("live_wanted")),
                            )
                            for event in h3_evictions
                        ).items()
                    )
                },
            },
        },
        "received_without_first_exact_audit": {
            "accepted_receives_during_h3": len(h3_accepted_receives),
            "unique_received_keys_during_h3": len(h3_received_keys),
            "unique_received_keys_with_no_first_exact_event_anywhere": len(
                never_first_exact_keys
            ),
            "note": (
                "The trace-wide count includes forecast, parent, stale, and "
                "off-screen work. The visual conclusion uses the on-screen "
                "cohort's accepted_receives_never_exact list above."
            ),
        },
        "hunt3_eight": {
            "keys": len(HUNT3_KEYS),
            "blocked_reason_counts": dict(
                Counter(
                    reason
                    for row in hunt_rows
                    for reason in str(row["blocked_reasons"]).split("+")
                    if reason
                )
            ),
            "wanted_count1500_evictions": sum(
                int(row["wanted_count1500_evictions"]) for row in hunt_rows
            ),
            "exact_drawn": sum(
                row["first_exact_draw_frame"] is not None for row in hunt_rows
            ),
        },
    }

    output_prefix.parent.mkdir(parents=True, exist_ok=True)
    partition_fields = [
        "key",
        "face",
        "level",
        "ix",
        "iy",
        "deep",
        "observed_frame",
        "observed_t_us",
        "projected_px_area",
        "gap_rank",
        "status",
        "missing_episode",
        "missing_frame",
        "missing_t_us",
        "draw_frame",
        "draw_t_us",
        "t_total_ms",
        "t_unadmitted_ms",
        "t_queue_ms",
        "t_build_ms",
        "t_land_swap_ms",
        "dominant_phase",
        "admit_sources",
        "worker_attempts",
        "worker_done_count",
        "reported_build_ms",
        "accepted_receives",
        "rejected_receives",
        "eviction_count",
        "eviction_reasons",
        "state_anomalies",
    ]
    write_csv(
        output_prefix.with_name(output_prefix.name + "-partition.csv"),
        [rounded(row) for row in rows],
        partition_fields,
    )
    write_csv(
        output_prefix.with_name(output_prefix.name + "-hunt3-eight.csv"),
        [rounded(row) for row in hunt_rows],
        list(hunt_rows[0].keys()),
    )
    summary_rows = [
        {
            key: event.get(key)
            for key in (
                "frame",
                "t_us",
                "wanted",
                "uncached",
                "covered_missing",
                "on_screen_missing",
                "on_screen_missing_px_area",
                "gap_pending",
                "gap_unadmitted",
                "exact_selected_draws",
                "pending_total",
                "productive_pending_total",
                "productive_workers_active",
            )
        }
        for event in h3_summaries
    ]
    write_csv(
        output_prefix.with_name(output_prefix.name + "-h3-summaries.csv"),
        summary_rows,
        list(summary_rows[0].keys()),
    )
    with output_prefix.with_name(
        output_prefix.name + "-summary.json"
    ).open("w", encoding="utf-8") as destination:
        json.dump(rounded(summary), destination, indent=2, sort_keys=True)
        destination.write("\n")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("trace", type=Path)
    parser.add_argument(
        "--output-prefix",
        required=True,
        type=Path,
        help="prefix for -partition.csv, -summary.json, and audit CSVs",
    )
    arguments = parser.parse_args()
    analyze(arguments.trace, arguments.output_prefix)


if __name__ == "__main__":
    main()
