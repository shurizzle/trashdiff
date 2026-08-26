(function () {
  var form = document.getElementById('admin-form');
  if (!form) return;
  function showErr(ref, msg) {
    var s = document.createElement('span');
    s.className = 'err';
    s.textContent = msg;
    ref.parentNode.insertBefore(s, ref.nextSibling);
  }
  function markBad(p, week) {
    var cb = p.querySelector('input[type=checkbox][value="' + week + '"]');
    cb.classList.add('bad');
    if (cb.nextElementSibling) cb.nextElementSibling.classList.add('bad');
  }
  function clearRowErr(p) {
    p.querySelectorAll('.bad').forEach(function (n) { n.classList.remove('bad'); });
    if (p.nextElementSibling && p.nextElementSibling.classList.contains('err')) {
      p.nextElementSibling.remove();
    }
  }
  function clearErrAfter(el) {
    if (el.nextElementSibling && el.nextElementSibling.classList.contains('err')) {
      el.nextElementSibling.remove();
    }
  }
  function renumber(day) {
    form.querySelectorAll('.row[data-day="' + day + '"]').forEach(function (p, i) {
      p.querySelectorAll('input[type=checkbox]').forEach(function (cb) {
        cb.name = day + '_weeks_' + i;
        cb.id = day + '_w' + i + '_' + cb.value;
        if (cb.nextElementSibling) cb.nextElementSibling.setAttribute('for', cb.id);
      });
      p.querySelector('input[type=text]').name = day + '_type_' + i;
      p.querySelector('button[name=del]').value = day + ':' + i;
    });
  }
  function rowHtml(day) {
    var h = '<p class="row" data-day="' + day + '"><span class="weeks">';
    for (var w = 1; w <= 5; w++) {
      h += '<input type="checkbox" id="' + day + '_w0_' + w + '" name="' + day +
        '_weeks_0" value="' + w + '"><label for="' + day + '_w0_' + w + '">' + w + '</label>';
    }
    h += '</span> <span class="field"><input type="text" name="' + day +
      '_type_0" value=""><button type="submit" name="del" value="' + day + ':0">-</button></span></p>';
    return h;
  }
  function addRow(day) {
    var tpl = document.createElement('div');
    tpl.innerHTML = rowHtml(day);
    var p = tpl.firstChild;
    var covered = {};
    form.querySelectorAll('.row[data-day="' + day + '"] input[type=checkbox]:checked')
      .forEach(function (cb) { covered[cb.value] = 1; });
    p.querySelectorAll('input[type=checkbox]').forEach(function (cb) {
      cb.checked = !covered[cb.value];
    });
    var rows = form.querySelectorAll('.row[data-day="' + day + '"]');
    var ref = rows.length
      ? rows[rows.length - 1]
      : form.querySelector('button[name=add][value="' + day + '"]').parentNode;
    ref.parentNode.insertBefore(p, ref.nextSibling);
    renumber(day);
  }
  form.addEventListener('click', function (e) {
    var add = e.target.closest('button[name=add]');
    if (add) { e.preventDefault(); addRow(add.value); return; }
    var del = e.target.closest('button[name=del]');
    if (del) {
      e.preventDefault();
      var p = del.closest('.row');
      clearRowErr(p);
      p.remove();
      renumber(del.value.split(':')[0]);
    }
  });
  form.addEventListener('change', function (e) {
    var t = e.target;
    if (t.closest('.row')) { clearRowErr(t.closest('.row')); }
    else if (t.name === 'pickup_time') { clearErrAfter(t.closest('label')); }
  });
  form.addEventListener('submit', function (e) {
    form.querySelectorAll('.bad').forEach(function (n) { n.classList.remove('bad'); });
    form.querySelectorAll('span.err').forEach(function (n) { n.remove(); });
    var errors = 0;
    var time = form.querySelector('input[name=pickup_time]');
    if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(time.value)) {
      showErr(time.closest('label'), I18N.errTime);
      errors++;
    }
    var days = {};
    form.querySelectorAll('.row').forEach(function (p) {
      var d = p.dataset.day;
      (days[d] = days[d] || []).push(p);
    });
    for (var day in days) {
      var seen = {};
      days[day].forEach(function (p) {
        var type = p.querySelector('input[type=text]').value.trim();
        var checked = [].slice.call(p.querySelectorAll('input[type=checkbox]:checked'));
        if (checked.length && !type) { showErr(p, I18N.errType); errors++; return; }
        if (!type) return;
        checked.forEach(function (cb) {
          if (seen[cb.value]) {
            markBad(seen[cb.value], cb.value);
            markBad(p, cb.value);
            showErr(p, I18N.errOverlap.replace('%s', I18N.days[day]).replace('%d', cb.value));
            errors++;
          } else { seen[cb.value] = p; }
        });
      });
    }
    if (errors) e.preventDefault();
  });
})();
