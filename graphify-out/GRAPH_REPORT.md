# Graph Report - audioInterface  (2026-11-09)

## Corpus Check
- 34 files · ~81,814 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 655 nodes · 1320 edges · 26 communities
- Extraction: 92% EXTRACTED · 8% INFERRED · 0% AMBIGUOUS · INFERRED: 108 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `85f4d857`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- [[_COMMUNITY_Community 0|Community 0]]
- [[_COMMUNITY_Community 1|Community 1]]
- [[_COMMUNITY_Community 2|Community 2]]
- [[_COMMUNITY_Community 3|Community 3]]
- [[_COMMUNITY_Community 4|Community 4]]
- [[_COMMUNITY_Community 5|Community 5]]
- [[_COMMUNITY_Community 6|Community 6]]
- [[_COMMUNITY_Community 7|Community 7]]
- [[_COMMUNITY_Community 8|Community 8]]
- [[_COMMUNITY_Community 9|Community 9]]
- [[_COMMUNITY_Community 10|Community 10]]
- [[_COMMUNITY_Community 11|Community 11]]
- [[_COMMUNITY_Community 12|Community 12]]
- [[_COMMUNITY_Community 13|Community 13]]
- [[_COMMUNITY_Community 14|Community 14]]
- [[_COMMUNITY_Community 15|Community 15]]
- [[_COMMUNITY_Community 16|Community 16]]
- [[_COMMUNITY_Community 17|Community 17]]
- [[_COMMUNITY_Community 18|Community 18]]
- [[_COMMUNITY_Community 19|Community 19]]
- [[_COMMUNITY_Community 20|Community 20]]
- [[_COMMUNITY_Community 21|Community 21]]
- [[_COMMUNITY_Community 22|Community 22]]
- [[_COMMUNITY_Community 23|Community 23]]

## God Nodes (most connected - your core abstractions)
1. `App` - 22 edges
2. `run()` - 22 edges
3. `Worker` - 21 edges
4. `WasapiBackend` - 21 edges
5. `Settings` - 20 edges
6. `addr()` - 18 edges
7. `CoreAudioBackend` - 18 edges
8. `PulseBackend` - 16 edges
9. `MeterEngine` - 15 edges
10. `Audio Interface: Build Plan` - 14 edges

## Surprising Connections (you probably didn't know these)
- `dev()` --calls--> `test_device()`  [INFERRED]
  src/rules.rs → src/model.rs
- `foreign_service_on_the_port_is_not_mistaken_for_us()` --calls--> `spawn()`  [INFERRED]
  src/single_instance.rs → src/audio_tools.rs
- `run()` --calls--> `speaker_test()`  [INFERRED]
  src/main.rs → src/audio_tools.rs
- `run()` --calls--> `mic_test()`  [INFERRED]
  src/main.rs → src/audio_tools.rs
- `run()` --calls--> `wait()`  [INFERRED]
  src/main.rs → src/audio_tools.rs

## Communities (26 total, 0 thin omitted)

### Community 0 - "Community 0"
Cohesion: 0.06
Nodes (24): add_f64(), Biquad, Consumer, from_db(), history_buckets_and_typical_level(), k_weighting_matches_the_ebu_reference(), KWeighting, level_cell_consumers_are_independent() (+16 more)

### Community 1 - "Community 1"
Cohesion: 0.07
Nodes (28): channel_scalars(), channel_volume(), channel_volume_values(), Conn, cstr(), is_io_error(), parse_stream_id(), prop() (+20 more)

### Community 2 - "Community 2"
Cohesion: 0.12
Nodes (18): ActiveRecording, auto_records_while_another_app_uses_the_mic(), detects_gain_lowered_by_another_app(), Event, EventKind, FinishedRecording, manual_recording_writes_files_and_report(), mock_factory() (+10 more)

### Community 3 - "Community 3"
Cohesion: 0.09
Nodes (29): analyze(), analyze_with(), AppContext, call(), csv_roundtrip(), drop_without_gain_change_is_acoustic(), Finding, finding_t() (+21 more)

### Community 4 - "Community 4"
Cohesion: 0.09
Nodes (17): device_id(), flow(), IPolicyConfig, mix_format(), prop_string(), prop_u32(), session_app_name(), session_instance_id() (+9 more)

### Community 5 - "Community 5"
Cohesion: 0.09
Nodes (33): run(), egui_icon(), ico_header_is_valid(), icon_has_transparent_corners_and_opaque_center(), lerp(), meter_color(), png_bytes(), render() (+25 more)

### Community 6 - "Community 6"
Cohesion: 0.15
Nodes (17): addr(), Asbd, AudioObjectPropertyAddress, check(), CoreAudioBackend, data_size(), get(), get_array() (+9 more)

### Community 7 - "Community 7"
Cohesion: 0.13
Nodes (14): ago(), App, draw_history(), draw_meter(), draw_spectrum(), GuiOptions, low_power_wgpu(), open_mic_privacy_settings() (+6 more)

### Community 8 - "Community 8"
Cohesion: 0.11
Nodes (14): aliases_and_hidden_devices(), auto_record_app_filter(), cycling_respects_preferences_and_hidden(), DeviceAlias, gain_lock_lookup_by_id_or_name(), GainLock, HiddenDevice, partial_and_corrupt_files() (+6 more)

### Community 9 - "Community 9"
Cohesion: 0.09
Nodes (16): AudioBackend, create(), read_csv(), Inputs, report(), report_contains_the_essentials(), format_labels(), test_device() (+8 more)

### Community 10 - "Community 10"
Cohesion: 0.11
Nodes (12): Channel, access_denied_gets_privacy_hint(), AudioTap, build(), explain_open_error(), find_device(), MeterEngine, Monitor (+4 more)

### Community 11 - "Community 11"
Cohesion: 0.18
Nodes (16): capture(), find(), h_set_recording(), mic_test(), monitor(), play(), resample(), resampling_keeps_duration_and_shape() (+8 more)

### Community 12 - "Community 12"
Cohesion: 0.11
Nodes (18): Build from source, Build from source on Linux without sudo, Build from source on Windows without admin, code:powershell (# 1. Install Rust for your user only (no admin). Download ru), code:sh (tar xzf audio-interface-*-linux-x86_64.tar.gz), code:sh (# 1. Rust, per user:), code:sh (# Debian / Ubuntu), code:sh (curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | ) (+10 more)

### Community 13 - "Community 13"
Cohesion: 0.2
Nodes (12): auto_switch_on_connect_only(), AutoSwitcher, best_preferred(), dev(), gain_corrections(), gain_lock_detects_drift_only(), gain_locker_accepts_quantized_volume(), gain_locker_exact_devices_are_not_marked() (+4 more)

### Community 14 - "Community 14"
Cohesion: 0.2
Nodes (7): Command, default_hotkeys_parse(), Desktop, parse_hotkey(), Tray, tray_state_lists_devices_in_cycle_order(), TrayState

### Community 15 - "Community 15"
Cohesion: 0.12
Nodes (16): 2026-11-08 (night), 2026-11-09, Audio Interface: Build Plan, Phase 0: Project setup, Phase 10: Which app is using the mic + auto-record calls, Phase 11: Professional feature set, Phase 1: Core abstractions (platform independent), Phase 2: Platform backends (+8 more)

### Community 16 - "Community 16"
Cohesion: 0.12
Nodes (15): Audio Interface, code:sh (git clone https://github.com/ikjbi/audioInterface), code:sh (sudo apt install pkg-config libasound2-dev libxkbcommon-dev ), code:text (audio-interface                          # open the app), code:block4 (┌────────────────────────── GUI (egui) / CLI ───────────────), code:sh (cargo test                     # unit tests (+ a read-only C), Command line, Development (+7 more)

### Community 17 - "Community 17"
Cohesion: 0.27
Nodes (10): Action, capture(), capture_then_plan_is_a_noop(), DeviceLevel, devs(), plan(), plan_restores_changes_and_reports_missing(), Profile (+2 more)

### Community 18 - "Community 18"
Cohesion: 0.24
Nodes (3): mock_set_default_is_exclusive(), mock_volume_clamps(), MockBackend

### Community 19 - "Community 19"
Cohesion: 0.17
Nodes (11): [0.1.0-alpha.1] - 2026-11-09, [0.1.1] - 2026-11-09, Added, Added, Added, Changed, Changed, Changelog (+3 more)

### Community 20 - "Community 20"
Cohesion: 0.44
Nodes (9): entries_reference_the_executable(), entry_path(), exe(), is_enabled(), linux_desktop(), macos_plist(), reg(), set_enabled() (+1 more)

### Community 21 - "Community 21"
Cohesion: 0.4
Nodes (4): bar_level(), levels_stay_in_range(), peak_level(), Splash

### Community 22 - "Community 22"
Cohesion: 0.52
Nodes (3): Analyzer, band_centres(), sine_lands_in_the_right_band_at_the_right_level()

### Community 23 - "Community 23"
Cohesion: 0.6
Nodes (3): cli(), devices(), find()

## Knowledge Gaps
- **76 isolated node(s):** `GainCorrection`, `Inputs`, `ToolState`, `RecordingInfo`, `Sample` (+71 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `run()` connect `Community 5` to `Community 3`, `Community 8`, `Community 9`, `Community 11`, `Community 17`?**
  _High betweenness centrality (0.203) - this node is a cross-community bridge._
- **Why does `test_device()` connect `Community 9` to `Community 1`, `Community 8`, `Community 13`, `Community 14`, `Community 17`?**
  _High betweenness centrality (0.161) - this node is a cross-community bridge._
- **Why does `resolve_device()` connect `Community 5` to `Community 1`?**
  _High betweenness centrality (0.099) - this node is a cross-community bridge._
- **Are the 13 inferred relationships involving `run()` (e.g. with `port()` and `signal_existing()`) actually correct?**
  _`run()` has 13 INFERRED edges - model-reasoned connections that need verification._
- **What connects `GainCorrection`, `Inputs`, `ToolState` to the rest of the system?**
  _76 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Community 0` be split into smaller, more focused modules?**
  _Cohesion score 0.06 - nodes in this community are weakly interconnected._
- **Should `Community 1` be split into smaller, more focused modules?**
  _Cohesion score 0.07 - nodes in this community are weakly interconnected._