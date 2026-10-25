// Collapse the navigation <details> on small screens; keep it open on wide ones.
// Without JS the menu stays open (see the `open` attribute in the shell).
(function () {
  var menu = document.querySelector(".nav-menu");
  if (!menu || !window.matchMedia) return;
  var mq = window.matchMedia("(max-width: 768px)");
  function sync(e) {
    if (e.matches) {
      menu.removeAttribute("open");
    } else {
      menu.setAttribute("open", "");
    }
  }
  sync(mq);
  mq.addEventListener("change", sync);
})();
