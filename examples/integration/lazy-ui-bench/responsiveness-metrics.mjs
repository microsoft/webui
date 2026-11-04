// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/** Serialized into the benchmark page; it intentionally has no outer dependencies. */
export function installResponsivenessProbe() {
  const supported = PerformanceObserver.supportedEntryTypes;
  if (!supported.includes('event') || !supported.includes('longtask')) {
    throw new Error('Event Timing and Long Tasks support are required.');
  }
  const events = [];
  const tasks = [];
  const frames = [];
  const actions = [];
  const eventSink = records => {
    for (const entry of records) events.push({
      name: entry.name, start: entry.startTime, duration: entry.duration,
      processingStart: entry.processingStart, processingEnd: entry.processingEnd,
      interactionId: entry.interactionId,
    });
  };
  const taskSink = records => {
    for (const entry of records) tasks.push({ start: entry.startTime, duration: entry.duration });
  };
  const eventObserver = new PerformanceObserver(list => eventSink(list.getEntries()));
  const taskObserver = new PerformanceObserver(list => taskSink(list.getEntries()));
  eventObserver.observe({ type: 'event', durationThreshold: 16, buffered: true });
  taskObserver.observe({ type: 'longtask', buffered: true });
  let previous;
  function frame(timestamp) {
    if (previous !== undefined) frames.push({ start: previous, duration: timestamp - previous });
    previous = timestamp;
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
  let phase;
  window.uiProbe = {
    actions,
    arm(name) { phase = name; },
    begin(event, pending) {
      if (!phase || !event.isTrusted) throw new Error('A named, trusted input event is required.');
      const action = {
        name: phase, event: event.type, start: event.timeStamp,
        handlerStart: performance.now(), trusted: event.isTrusted, pending,
      };
      phase = undefined;
      actions.push(action);
      return action;
    },
    finish(action) {
      action.domReady = performance.now();
      requestAnimationFrame(() => requestAnimationFrame(() => {
        action.nextFrame = performance.now();
        action.complete = true;
      }));
    },
    read() {
      eventSink(eventObserver.takeRecords());
      taskSink(taskObserver.takeRecords());
      return actions.map(action => {
        const timing = events.find(entry =>
          entry.name === action.event && Math.abs(entry.start - action.start) < 1);
        const overlaps = entry =>
          entry.start < action.nextFrame && entry.start + entry.duration > action.start;
        const phaseTasks = tasks.filter(overlaps);
        const phaseFrames = frames.filter(entry =>
          entry.start >= action.start && entry.start < action.nextFrame);
        return {
          name: action.name, trusted: action.trusted, pendingAtInput: action.pending,
          eventTimestamp: action.start,
          inputDelayMs: action.handlerStart - action.start,
          domReadyMs: action.domReady - action.start,
          twoRafReadyMs: action.nextFrame - action.start,
          eventTimingMs: timing?.duration ?? null,
          eventTiming: timing ?? null,
          maxLongTaskMs: Math.max(0, ...phaseTasks.map(entry => entry.duration)),
          longTaskCount: phaseTasks.length,
          maxFrameGapMs: Math.max(0, ...phaseFrames.map(entry => entry.duration)),
          frameGapsOver50Ms: phaseFrames.filter(entry => entry.duration > 50).length,
        };
      });
    },
  };
}

/** Missing Event Timing entries are censored below 16 ms, never silently dropped or zero-filled. */
export function summarize(values, censorBelowMs) {
  if (!values.length) throw new Error('Cannot summarize an empty sample.');
  if (values.some(value => value !== null && (!Number.isFinite(value) || value < 0))) {
    throw new Error('Samples must be nonnegative finite numbers.');
  }
  if (values.includes(null) && !(censorBelowMs > 0)) throw new Error('Missing samples need a censor bound.');
  const ordered = values.map(value => ({
    value: value ?? censorBelowMs, censored: value === null,
  })).sort((a, b) => a.value - b.value || Number(b.censored) - Number(a.censored));
  const quantile = fraction => ordered[Math.ceil(ordered.length * fraction) - 1];
  return {
    count: values.length, belowThreshold: values.filter(value => value === null).length,
    median: quantile(0.5), p95: quantile(0.95), worst: ordered.at(-1),
  };
}
