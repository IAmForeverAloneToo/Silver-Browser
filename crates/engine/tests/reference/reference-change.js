// The changes a reference page asks for, made for its capture (M22).
//
// <meta name="reference-change" content="ID ATTRIBUTE VALUE; ..."> names
// each change: the element of that id given that attribute, the value
// being the rest of the change. <meta name="reference-time"
// content="T1 T2 ... DRAW"> gives the time of each change and of the
// drawing, in milliseconds from the first frame.
//
// Each animation is held at each change's time, the change made and the
// style computed, so that the transitions it starts begin there; then
// every animation is held at the drawing's time, the moment the capture
// shows. A CSS animation's time counts from the first frame, a
// transition's from the change that started it; one the page pauses
// keeps its own. tests/reference.rs makes the same changes through the
// engine between frames drawn at the same times.
addEventListener('load', () => {
  const meta = name => document.querySelector(`meta[name="${name}"]`).content;
  const changes = meta('reference-change')
    .split(';')
    .map(change => change.trim().split(/\s+/));
  const times = meta('reference-time').trim().split(/\s+/).map(Number);
  const started = new Map();
  const begin = at => {
    for (const animation of document.getAnimations()) {
      if (!started.has(animation)) {
        started.set(animation, animation.playState === 'paused' ? null : at);
      }
    }
  };
  const hold = at => {
    for (const animation of document.getAnimations()) {
      const start = started.get(animation);
      if (start !== null) {
        animation.pause();
        animation.currentTime = at - start;
      }
    }
  };
  document.body.offsetWidth;
  begin(0);
  changes.forEach(([id, name, ...value], i) => {
    hold(times[i]);
    document.getElementById(id).setAttribute(name, value.join(' '));
    document.body.offsetWidth;
    begin(times[i]);
  });
  hold(times[times.length - 1]);
});
