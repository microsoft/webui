# Contact Book macOS desktop benchmark

These results compare release Electron and WebUI desktop hosts using the same
frozen Contact Book application bundle, state, theme, 1200x800 viewport, and
hydrated Dashboard readiness contract. Each host received one warmup followed
by 20 alternating measured pairs with fresh processes and profiles.

The analyzer applies the modified z-score independently to each host for each
metric. A pair is excluded when either host has an absolute score above 3.5.
No samples are removed manually.

Reproduce the checked-in summary from the raw cohort:

```bash
python3 benchmarks/macos/contact-book/analyze.py \
  benchmarks/macos/contact-book/results/macos-raw.json \
  benchmarks/macos/contact-book/results/macos-summary.json
```

The raw lifecycle exposes Dashboard readiness, reliable buffered
first-contentful-paint timestamps, host-process RSS, child-process CPU, and
close-to-exit. It does not expose an equivalent process-start-to-visible-window
milestone. Electron emits its `Ready` receipt before `BrowserWindow`
construction, while the native `Ready` event is a host lifecycle callback and
not proof that pixels were composited. The summary therefore reports that
metric as unavailable rather than substituting either event.

RSS covers the host process only. Native WebKit XPC helper attribution remains
incomplete, so the RSS comparison is directional rather than whole-application
memory.
