/* Loaded only by `code-rcl serve`. Holds one EventSource open to the local
   server; when this tab closes the connection drops and the server exits.
   An open EventSource is not subject to background-tab timer throttling, so a
   backgrounded tab keeps the server alive. */
(function () {
  "use strict";
  var statusEl = document.getElementById("status");
  function show(msg) {
    if (statusEl) {
      statusEl.textContent = msg;
      statusEl.classList.add("show");
    }
  }
  function hide() {
    if (statusEl) statusEl.classList.remove("show");
  }
  var es = new EventSource("/live");
  es.addEventListener("open", hide);
  es.addEventListener("error", function () {
    // EventSource auto-reconnects; if the server is really gone this just keeps firing.
    show("server stopped — the graph is now static (rerun `code-rcl serve`)");
  });
})();
