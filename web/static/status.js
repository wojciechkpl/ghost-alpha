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
