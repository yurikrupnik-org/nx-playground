(() => {
  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));

  /* theme ------------------------------------------------------------- */
  const root = document.documentElement;
  const stored = localStorage.getItem('monodocs-theme');
  if (stored) root.dataset.theme = stored;
  const themeButton = $('#theme');
  const paintTheme = () => {
    themeButton.textContent =
      root.dataset.theme === 'light' ? 'Dark theme' : 'Light theme';
  };
  paintTheme();
  themeButton.addEventListener('click', () => {
    root.dataset.theme = root.dataset.theme === 'light' ? 'dark' : 'light';
    localStorage.setItem('monodocs-theme', root.dataset.theme);
    paintTheme();
  });

  /* expand / collapse supplementary docs and API modules ---------------- */
  const expandButton = $('#expand');
  const details = $$('details.extra, details.api-mod');
  const paintExpand = () => {
    const allOpen = details.length > 0 && details.every((d) => d.open);
    expandButton.textContent = allOpen
      ? 'Collapse all docs'
      : 'Expand all docs';
    expandButton.dataset.state = allOpen ? 'open' : 'closed';
  };
  if (details.length === 0) {
    expandButton.remove();
  } else {
    paintExpand();
    expandButton.addEventListener('click', () => {
      const open = expandButton.dataset.state !== 'open';
      details.forEach((d) => {
        d.open = open;
      });
      paintExpand();
    });
    details.forEach((d) => d.addEventListener('toggle', paintExpand));
  }

  /* copy buttons -------------------------------------------------------- */
  $$('figure.code .copy').forEach((button) => {
    button.addEventListener('click', async () => {
      const code = $('code', button.closest('figure.code')).innerText;
      try {
        await navigator.clipboard.writeText(code);
        button.textContent = 'copied';
      } catch {
        button.textContent = 'failed';
      }
      setTimeout(() => {
        button.textContent = 'copy';
      }, 1200);
    });
  });

  /* filter -------------------------------------------------------------- */
  const filter = $('#filter');
  const empty = $('#empty');
  const sections = $$('section.project').map((el) => ({
    el,
    nav: $(`.nav-item[data-slug="${el.dataset.slug}"]`),
    // textContent, not innerText: a name inside a collapsed <details> is still searchable.
    haystack: (el.dataset.name + ' ' + el.textContent).toLowerCase(),
  }));

  const applyFilter = () => {
    const query = filter.value.trim().toLowerCase();
    let shown = 0;
    sections.forEach(({ el, nav, haystack }) => {
      const hit = query === '' || haystack.includes(query);
      el.classList.toggle('hidden', !hit);
      if (nav) nav.classList.toggle('hidden', !hit);
      if (hit) shown += 1;
    });
    $$('.nav-group').forEach((group) => {
      const any = $$('.nav-item', group).some(
        (item) => !item.classList.contains('hidden'),
      );
      group.classList.toggle('hidden', !any);
    });
    empty.hidden = shown > 0;
  };

  filter.addEventListener('input', applyFilter);
  filter.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') {
      filter.value = '';
      applyFilter();
      filter.blur();
    }
  });
  document.addEventListener('keydown', (event) => {
    if (event.key === '/' && document.activeElement !== filter) {
      event.preventDefault();
      filter.focus();
      filter.select();
    }
  });

  /* open collapsed docs when a link targets something inside them --------- */
  const revealHash = () => {
    const id = decodeURIComponent(location.hash.slice(1));
    if (!id) return;
    const target = document.getElementById(id);
    if (!target) return;
    let parent = target.closest('details');
    while (parent) {
      parent.open = true;
      parent = parent.parentElement
        ? parent.parentElement.closest('details')
        : null;
    }
    target.scrollIntoView();
  };
  window.addEventListener('hashchange', revealHash);
  revealHash();

  /* scroll spy ------------------------------------------------------------ */
  const navLinks = new Map();
  $$('.nav-sub a, .nav-item > a').forEach((link) => {
    navLinks.set(link.getAttribute('href').slice(1), link);
  });
  const headings = $$('.doc-h').filter((h) => navLinks.has(h.id));
  const observer = new IntersectionObserver(
    (entries) => {
      entries
        .filter((entry) => entry.isIntersecting)
        .forEach((entry) => {
          navLinks.forEach((link) => link.classList.remove('active'));
          $$('.nav-item').forEach((item) => item.classList.remove('active'));
          const link = navLinks.get(entry.target.id);
          if (!link) return;
          link.classList.add('active');
          const item = link.closest('.nav-item');
          if (item) item.classList.add('active');
        });
    },
    { rootMargin: '0px 0px -75% 0px', threshold: 0 },
  );
  headings.forEach((heading) => observer.observe(heading));
})();
