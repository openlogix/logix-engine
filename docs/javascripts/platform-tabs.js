// Opens the platform tabs (macOS, Windows, Linux) on the reader's own
// system each time a page opens, even if they chose another tab before
// (the user's decision, 2026-11-20: a remembered Windows choice opened the
// tutorial on Windows on a Mac). A click still switches every tab while
// reading (content.tabs.link). Phones and unknown systems keep what MkDocs
// Material shows.
(function () {
  function readerPlatform() {
    var ua = (navigator.userAgent || "").toLowerCase();
    if (/android|iphone|ipad/.test(ua)) return null;
    var p = ((navigator.userAgentData && navigator.userAgentData.platform) ||
      navigator.platform || ua).toLowerCase();
    if (p.indexOf("mac") >= 0) return "macOS";
    if (p.indexOf("win") >= 0) return "Windows";
    if (p.indexOf("linux") >= 0 || p.indexOf("x11") >= 0) return "Linux";
    return null;
  }
  function select() {
    var name = readerPlatform();
    if (!name) return;
    var labels = document.querySelectorAll(".tabbed-labels > label");
    for (var i = 0; i < labels.length; i++) {
      if (labels[i].textContent.trim() !== name) continue;
      var input = document.getElementById(labels[i].getAttribute("for"));
      if (input && input.checked) return;
      // A click, so Material links every tab set and moves its indicator;
      // keep the page where it is.
      var x = window.scrollX, y = window.scrollY;
      labels[i].click();
      window.scrollTo(x, y);
      return;
    }
  }
  // After Material has restored a remembered tab.
  window.addEventListener("load", function () { setTimeout(select, 0); });
})();
