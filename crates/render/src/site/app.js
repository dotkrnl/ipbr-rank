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
        Array.prototype.forEach.call(headers, function (h) {
          h.removeAttribute('data-sort-active');
          h.setAttribute('aria-sort', 'none');
        });
        if (alreadyActive) {
          // Restore default order — only "row" rows; expand rows follow their parent.
          relayout(table, defaultOrder);
          syncTable(table);
          return;
        }
        th.setAttribute('data-sort-active', 'desc');
        th.setAttribute('aria-sort', 'descending');
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
        syncTable(table);
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

  // === Rank column, leader tick, and active-column wash ===
  // The `#` column is a position marker (1..N of the current view), and the
  // leader + column wash follow whichever score column drives the ordering.
  // Default ordering is build, so an inactive header resolves to 'b'.
  function isScoreCol(key) { return key === 'i' || key === 'p' || key === 'b' || key === 'r'; }
  function currentSortKey(table) {
    var active = table.querySelector('th[data-sort-active="desc"]');
    return active ? active.getAttribute('data-sort') : 'b';
  }
  function syncTable(table) {
    if (!table) return;
    var key = currentSortKey(table);
    if (isScoreCol(key)) table.setAttribute('data-active-col', key);
    else table.removeAttribute('data-active-col');
    var rows = table.tBodies[0].rows;
    var leaderKey = isScoreCol(key) ? key : null;
    var n = 0, leaderDone = false;
    Array.prototype.forEach.call(rows, function (r) {
      if (!r.classList || !r.classList.contains('row')) return;
      r.classList.remove('leader');
      if (r.hidden) return;
      n += 1;
      var cell = r.querySelector('td.rank');
      if (cell) cell.textContent = n;
      if (leaderKey && !leaderDone) { r.classList.add('leader'); leaderDone = true; }
    });
  }

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
      syncTable(table);
    }

    input.addEventListener('input', function () {
      state.text = input.value.toLowerCase();
      apply();
    });
    Array.prototype.forEach.call(chips, function (chip) {
      chip.addEventListener('click', function () {
        Array.prototype.forEach.call(chips, function (c) {
          c.classList.remove('active');
          c.setAttribute('aria-pressed', 'false');
        });
        chip.classList.add('active');
        chip.setAttribute('aria-pressed', 'true');
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
    var btn = cell && cell.querySelector('button');
    if (btn) {
      btn.textContent = open ? '▾' : '▸';
      btn.setAttribute('aria-expanded', open ? 'true' : 'false');
      var label = open ? btn.getAttribute('data-label-open') : btn.getAttribute('data-label-closed');
      if (label) btn.setAttribute('aria-label', label);
    }
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
    syncTable(document.getElementById('leaderboard-table'));
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
