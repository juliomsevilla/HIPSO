(function () {
  var raiz = document.body.getAttribute('data-raiz') || '';
  var d = document;

  // tema claro u oscuro
  d.getElementById('tema').addEventListener('click', function () {
    var el = d.documentElement;
    var oscuro = el.getAttribute('data-theme') === 'dark' ||
      (!el.getAttribute('data-theme') && window.matchMedia('(prefers-color-scheme: dark)').matches);
    var nuevo = oscuro ? 'light' : 'dark';
    el.setAttribute('data-theme', nuevo);
    try { localStorage.setItem('hipso-tema', nuevo); } catch (e) {}
  });

  // menú lateral en pantallas estrechas
  var lat = d.getElementById('lateral');
  d.getElementById('abrir-menu').addEventListener('click', function () { lat.classList.toggle('visible'); });
  lat.addEventListener('click', function (e) { if (e.target.tagName === 'A') lat.classList.remove('visible'); });
  var act = lat.querySelector('a.activa');
  if (act && act.scrollIntoView) act.scrollIntoView({ block: 'center' });

  // botón de copiar en los bloques de código
  d.querySelectorAll('article pre').forEach(function (pre) {
    var b = d.createElement('button');
    b.className = 'copiar'; b.textContent = 'Copiar';
    b.addEventListener('click', function () {
      var t = pre.querySelector('code') ? pre.querySelector('code').innerText : pre.innerText;
      if (navigator.clipboard) navigator.clipboard.writeText(t).then(function () { b.textContent = 'Copiado'; setTimeout(function () { b.textContent = 'Copiar'; }, 1500); });
    });
    pre.appendChild(b);
  });

  // búsqueda en el índice generado al construir la página
  var cuadro = d.getElementById('busqueda'), campo = d.getElementById('consulta'), lista = d.getElementById('resultados');
  var indice = null, sel = 0, actuales = [];
  function norm(t) { return t.normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase(); }
  function cargar(cb) {
    if (indice) return cb();
    fetch(raiz + 'static/busqueda.json').then(function (r) { return r.json(); }).then(function (j) {
      indice = j.map(function (r) { r.n = norm(r.t + ' ' + r.p); r.c = norm(r.x); return r; }); cb();
    });
  }
  function abrir() { cuadro.hidden = false; campo.value = ''; lista.innerHTML = ''; campo.focus(); cargar(function () {}); }
  function cerrar() { cuadro.hidden = true; }
  function esc(t) { return t.replace(/[&<>]/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]; }); }
  function pintar() {
    lista.innerHTML = actuales.length ? '' : '<p>Sin resultados.</p>';
    actuales.forEach(function (r, i) {
      var a = d.createElement('a');
      a.href = raiz + r.u; if (i === sel) a.className = 'sel';
      a.innerHTML = '<b>' + esc(r.t) + '</b><small>' + esc(r.p) + (r.frag ? ' · ' + esc(r.frag) : '') + '</small>';
      lista.appendChild(a);
    });
  }
  function buscar() {
    var q = norm(campo.value.trim());
    if (!q || !indice) { actuales = []; lista.innerHTML = ''; return; }
    var palabras = q.split(/\s+/);
    var pun = [];
    indice.forEach(function (r) {
      var s = 0;
      for (var i = 0; i < palabras.length; i++) {
        var p = palabras[i], en_t = r.n.indexOf(p) >= 0, en_c = r.c.indexOf(p) >= 0;
        if (!en_t && !en_c) { s = 0; break; }
        s += (en_t ? 10 : 0) + (en_c ? 2 : 0);
        if (r.n.indexOf(q) >= 0) s += 6;
      }
      if (s > 0) {
        var pos = r.c.indexOf(palabras[0]);
        var fr = pos >= 0 ? r.x.substr(Math.max(0, pos - 30), 110) : '';
        pun.push({ s: s, r: Object.assign({ frag: fr }, r) });
      }
    });
    pun.sort(function (a, b) { return b.s - a.s; });
    actuales = pun.slice(0, 12).map(function (x) { return x.r; }); sel = 0; pintar();
  }
  d.getElementById('abrir-busqueda').addEventListener('click', abrir);
  campo.addEventListener('input', function () { cargar(buscar); });
  cuadro.addEventListener('click', function (e) { if (e.target === cuadro) cerrar(); });
  d.addEventListener('keydown', function (e) {
    var tag = (d.activeElement && d.activeElement.tagName) || '';
    if ((e.key === '/' || (e.key === 'k' && (e.ctrlKey || e.metaKey))) && tag !== 'INPUT') { e.preventDefault(); abrir(); }
    else if (e.key === 'Escape') cerrar();
    else if (!cuadro.hidden && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
      e.preventDefault(); sel = Math.max(0, Math.min(actuales.length - 1, sel + (e.key === 'ArrowDown' ? 1 : -1))); pintar();
    } else if (!cuadro.hidden && e.key === 'Enter' && actuales[sel]) { location.href = raiz + actuales[sel].u; }
  });
})();
