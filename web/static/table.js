// Client-side sorting and filtering for tables (progressive enhancement).
//  - Sort:   <th data-sort="text|number"><button type="button">..</button></th>
//  - Filter: <input type="search" data-filter-for="TABLE_ID">
// State is kept in aria-sort / the input value, so it is re-applied after
// htmx swaps rows into a table body.
(function () {
  // Parse the first numeric token: thousands separators, U+2212 minus,
  // accounting parentheses "(1,234)" as negative, and K/M/B/T suffixes.
  var NUMBER = /[-+\u2212]?\d[\d,]*\.?\d*/;
  var MAGNITUDE = { K: 1e3, M: 1e6, B: 1e9, T: 1e12 };

  function toNumber(s) {
    var m = NUMBER.exec(s);
    if (!m) return null;
    var token = m[0].replace(/\u2212/g, "-").replace(/,/g, "");
    var n = parseFloat(token);
    if (isNaN(n)) return null;
    var after = s.charAt(m.index + m[0].length).toUpperCase();
    if ("KMBT".indexOf(after) !== -1 && after !== "") n *= MAGNITUDE[after];
    var before = s.slice(0, m.index).trim();
    if (before.slice(-1) === "(" && n > 0) n = -n;
    return n;
  }

  // data-sort-value on a cell wins over its visible text.
  function sortKey(row, i) {
    var c = row.cells[i];
    if (!c) return "";
    var v = c.getAttribute("data-sort-value");
    return v !== null ? v : c.textContent.trim();
  }

  function applySort(th) {
    var dir = th.getAttribute("aria-sort");
    if (dir !== "ascending" && dir !== "descending") return;
    var table = th.closest("table");
    var body = table && table.tBodies[0];
    if (!body) return;
    var idx = th.cellIndex;
    var numeric = th.getAttribute("data-sort") === "number";
    var sign = dir === "ascending" ? 1 : -1;
    var rows = Array.prototype.slice.call(body.rows);
    rows.sort(function (a, b) {
      var x = sortKey(a, idx);
      var y = sortKey(b, idx);
      if (numeric) {
        var nx = toNumber(x);
        var ny = toNumber(y);
        // Non-numeric cells always sort last, in both directions.
        if (nx === null && ny === null) return 0;
        if (nx === null) return 1;
        if (ny === null) return -1;
        return (nx - ny) * sign;
      }
      return x.localeCompare(y, undefined, { sensitivity: "base" }) * sign;
    });
    rows.forEach(function (r) {
      body.appendChild(r);
    });
  }

  function applyFilter(input) {
    var table = document.getElementById(input.getAttribute("data-filter-for"));
    var body = table && table.tBodies[0];
    if (!body) return;
    var q = input.value.trim().toLowerCase();
    Array.prototype.forEach.call(body.rows, function (r) {
      r.hidden = q !== "" && r.textContent.toLowerCase().indexOf(q) === -1;
    });
  }

  function reapply() {
    Array.prototype.forEach.call(
      document.querySelectorAll("th[data-sort][aria-sort]"),
      applySort
    );
    Array.prototype.forEach.call(
      document.querySelectorAll("input[data-filter-for]"),
      applyFilter
    );
  }

  document.addEventListener("click", function (e) {
    var button = e.target.closest ? e.target.closest("th[data-sort] button") : null;
    if (!button) return;
    var th = button.closest("th");
    var next = th.getAttribute("aria-sort") === "ascending" ? "descending" : "ascending";
    Array.prototype.forEach.call(th.parentNode.children, function (other) {
      other.removeAttribute("aria-sort");
    });
    th.setAttribute("aria-sort", next);
    applySort(th);
  });

  document.addEventListener("input", function (e) {
    if (e.target.matches && e.target.matches("input[data-filter-for]")) {
      applyFilter(e.target);
    }
  });

  document.addEventListener("htmx:afterSwap", reapply);
})();
