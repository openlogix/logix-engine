// Opens the platform tabs (macOS, Windows, Linux) on the reader's own
// system, until they choose a tab themselves: MkDocs Material remembers a
// chosen tab (content.tabs.link, in localStorage under a key ending in
// "__tabs"), and then this does nothing. Phones and unknown systems keep
// the first tab.
(function () {
  function readerPlatform() {
    var p = ((navigator.userAgentData && navigator.userAgentData.platform) ||
      navigator.platform || navigator.userAgent || "").toLowerCase();
    var ua = (navigator.userAgent || "").toLowerCase();
    if (/android|iphone|ipad/.test(ua)) return null;
    if (p.indexOf("mac") >= 0) return "macOS";
    if (p.indexOf("win") >= 0) return "Windows";
    if (p.indexOf("linux") >= 0 || p.indexOf("x11") >= 0) return "Linux";
    return null;
  }
  function chosenBefore() {
    try {
      for (var i = 0; i < localStorage.length; i++) {
        if (/__tabs$/.test(localStorage.key(i))) return true;
      }
    } catch (e) {}
    return false;
  }
  function select() {
    var name = readerPlatform();
    if (!name || chosenBefore()) return;
    document.querySelectorAll(".tabbed-labels > label").forEach(function (label) {
      if (label.textContent.trim() !== name) return;
      var input = document.getElementById(label.getAttribute("for"));
      if (input) input.checked = true;
    });
  }
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", select);
  } else {
    select();
  }
})();
