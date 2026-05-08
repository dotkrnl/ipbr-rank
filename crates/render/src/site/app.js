(function () {
  'use strict';

  // === Sort: descending only, click again returns to default order ===
  function initSort() {
    var table = document.getElementById('leaderboard-table');
    if (!table) return;
    var defaultOrder = Array.prototype.map.call(table.tBodies[0].rows, function (row) { return row; });
    var headers = table.querySelectorAll('th[data-sort]');
    Array.prototype.forEach.call(headers, function (th) {
      var btn = th.querySelector('button.sort');
      if (!btn) return;
      btn.addEventListener('click', function () {
        var key = th.getAttribute('data-sort');
        var alreadyActive = th.getAttribute('data-sort-active') === 'desc';
        Array.prototype.forEach.call(headers, function (h) { h.removeAttribute('data-sort-active'); });
        if (alreadyActive) {
          // Restore default order — only "row" rows; expand rows follow their parent.
          relayout(table, defaultOrder);
          return;
        }
        th.setAttribute('data-sort-active', 'desc');
        var rows = Array.prototype.filter.call(table.tBodies[0].rows, function (r) {
          return r.classList.contains('row');
        });
        rows.sort(function (a, b) {
          var av = sortValue(a, key);
          var bv = sortValue(b, key);
          if (av === bv) return 0;
          return av > bv ? -1 : 1; // DESC only
        });
        relayout(table, rows);
      });
    });
  }
  function sortValue(row, key) {
    var attr = row.getAttribute('data-sort-' + key);
    if (attr === null) return -Infinity;
    var n = parseFloat(attr);
    if (!isNaN(n)) return n;
    return attr.toLowerCase();
  }
  function relayout(table, orderedRows) {
    var tbody = table.tBodies[0];
    orderedRows.forEach(function (row) {
      var id = row.id;
      var expand = id ? tbody.querySelector('tr.expand[data-row="' + cssEscape(id) + '"]') : null;
      tbody.appendChild(row);
      if (expand) tbody.appendChild(expand);
    });
  }
  function cssEscape(s) { return s.replace(/(["\\])/g, '\\$1'); }

  // === Filter (text + vendor chips) ===
  function initFilter() {
    var input = document.querySelector('[data-filter-input]');
    var table = input && document.querySelector(input.getAttribute('data-filter-input'));
    if (!input || !table) return;
    var chips = document.querySelectorAll('.vendor-chips [data-vendor]');
    var state = { text: '', vendor: '' };

    function apply() {
      Array.prototype.forEach.call(table.tBodies[0].rows, function (row) {
        if (!row.classList.contains('row')) return;
        var text = row.textContent.toLowerCase();
        var vendor = row.getAttribute('data-vendor') || '';
        var matchText = !state.text || text.indexOf(state.text) !== -1;
        var matchVendor = !state.vendor || state.vendor === vendor;
        var visible = matchText && matchVendor;
        row.hidden = !visible;
        var expand = row.id ? table.tBodies[0].querySelector('tr.expand[data-row="' + cssEscape(row.id) + '"]') : null;
        if (expand && !visible) {
          expand.hidden = true;
          expand.classList.remove('open');
          row.classList.remove('expanded');
        } else if (expand) {
          expand.hidden = false;
        }
      });
    }

    input.addEventListener('input', function () {
      state.text = input.value.toLowerCase();
      apply();
    });
    Array.prototype.forEach.call(chips, function (chip) {
      chip.addEventListener('click', function () {
        Array.prototype.forEach.call(chips, function (c) { c.classList.remove('active'); });
        chip.classList.add('active');
        state.vendor = chip.getAttribute('data-vendor') || '';
        apply();
      });
    });
  }

  // === Expand rows ===
  function initExpand() {
    var table = document.getElementById('leaderboard-table');
    if (!table) return;
    table.addEventListener('click', function (e) {
      var cell = e.target.closest && e.target.closest('td.expand-toggle');
      if (!cell) return;
      var row = cell.parentElement;
      toggleExpand(row);
    });
  }
  function toggleExpand(row) {
    if (!row || !row.classList.contains('row')) return;
    var table = row.closest('table');
    if (!table) return;
    var id = row.id;
    var expand = id ? table.tBodies[0].querySelector('tr.expand[data-row="' + cssEscape(id) + '"]') : null;
    if (!expand) return;
    var open = expand.classList.toggle('open');
    row.classList.toggle('expanded', open);
    var cell = row.querySelector('td.expand-toggle');
    if (cell) cell.textContent = open ? '▾' : '▸';
  }

  // === Anchor auto-expand ===
  function initAnchor() {
    if (!location.hash) return;
    var id = decodeURIComponent(location.hash.slice(1));
    var row = document.getElementById(id);
    if (!row) return;
    toggleExpand(row);
    row.scrollIntoView({ block: 'center' });
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', boot);
  } else {
    boot();
  }
  function boot() {
    initSort();
    initFilter();
    initExpand();
    initAnchor();
    initLocalTime();
  }

  // === Local time conversion for <time data-local-time> elements ===
  function initLocalTime() {
    var nodes = document.querySelectorAll('time[data-local-time]');
    Array.prototype.forEach.call(nodes, function (el) {
      var iso = el.getAttribute('datetime');
      if (!iso) return;
      var d = new Date(iso);
      if (isNaN(d.getTime())) return;
      var pad = function (n) { return n < 10 ? '0' + n : '' + n; };
      el.textContent = d.getFullYear() + '-' + pad(d.getMonth() + 1) + '-' + pad(d.getDate())
        + ' ' + pad(d.getHours()) + ':' + pad(d.getMinutes());
    });
  }
})();
