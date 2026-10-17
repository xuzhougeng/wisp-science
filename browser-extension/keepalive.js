// Liveness bookkeeping for the bridge WebSocket.
//
// A half-open TCP connection (app restarted while Chrome's network service
// kept the socket, sleep/resume, a dropped NAT entry) still reports
// `readyState === OPEN`: send() buffers forever, no close frame ever arrives,
// and `onclose` never fires. The service worker therefore tracks when Wisp
// last sent *anything* and treats a long silence as a dead socket that must be
// closed by hand so connect() can rebuild it. Wisp answers every JSON ping
// with a pong, so a healthy connection is never silent for long.
function createLivenessTracker(options) {
  var opts = options || {};
  var staleAfterMs = opts.staleAfterMs || 90000;
  var now = opts.now || function () { return Date.now(); };
  var lastActivity = 0;
  return {
    // Call when a socket opens: the handshake counts as activity.
    reset: function () { lastActivity = now(); },
    // Call for every inbound frame, whatever its type.
    touch: function () { lastActivity = now(); },
    // True once Wisp has been silent longer than the stale window.
    isStale: function () { return lastActivity > 0 && now() - lastActivity > staleAfterMs; },
    lastActivity: function () { return lastActivity; },
    staleAfterMs: staleAfterMs
  };
}

if (typeof self !== "undefined") {
  self.createLivenessTracker = createLivenessTracker;
}
