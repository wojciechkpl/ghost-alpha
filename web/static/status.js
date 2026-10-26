// Announce user-triggered htmx updates in the page's [role=status] region.
// Polled and load-time swaps stay silent so screen readers are not interrupted.
document.addEventListener("htmx:afterSwap", function (e) {
  var cfg = e.detail && e.detail.requestConfig;
  var trigger = cfg && cfg.triggeringEvent;
  var type = trigger && trigger.type;
  if (type !== "change" && type !== "submit" && type !== "click") return;
  var region = document.querySelector("[role=status]");
  if (!region) return;
  region.textContent = "Updated " + new Date().toLocaleTimeString();
});

// The concentration page lazy-loads #etf-overlap. htmx drops 4xx/5xx bodies by
// default, which would leave the placeholder "loading" forever; swap the
// server's error region in instead.
document.addEventListener("htmx:beforeSwap", function (e) {
  var d = e.detail;
  if (!d || !d.target || d.target.id !== "etf-overlap") return;
  if (d.xhr && d.xhr.status >= 400) {
    d.shouldSwap = true;
    d.isError = false;
  }
});
