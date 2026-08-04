// Renderd Docs — shared JS
(function () {
  'use strict';

  // ── Theme ─────────────────────────────────────────────────
  const THEME_KEY = 'renderd-theme';
  const prefersDark = window.matchMedia('(prefers-color-scheme: dark)');

  function applyTheme(theme) {
    document.documentElement.setAttribute('data-theme', theme);
    const icon = document.getElementById('theme-icon');
    if (icon) icon.textContent = theme === 'dark' ? '🌙' : '☀️';
  }

  function getTheme() {
    const stored = localStorage.getItem(THEME_KEY);
    if (stored) return stored;
    return prefersDark.matches ? 'dark' : 'light';
  }

  function toggleTheme() {
    const current = document.documentElement.getAttribute('data-theme') || 'dark';
    const next = current === 'dark' ? 'light' : 'dark';
    localStorage.setItem(THEME_KEY, next);
    applyTheme(next);
  }

  // Apply theme before paint
  applyTheme(getTheme());

  document.addEventListener('DOMContentLoaded', function () {
    // Re-apply in case DOM wasn't ready
    applyTheme(getTheme());

    // Theme toggle button
    const toggleBtn = document.getElementById('theme-toggle');
    if (toggleBtn) toggleBtn.addEventListener('click', toggleTheme);

    // ── Mobile nav ───────────────────────────────────────────
    const hamburger = document.getElementById('nav-hamburger');
    const mobileNav = document.getElementById('nav-mobile');
    if (hamburger && mobileNav) {
      hamburger.addEventListener('click', function () {
        mobileNav.classList.toggle('open');
      });
      // Close on link click
      mobileNav.querySelectorAll('a').forEach(function (a) {
        a.addEventListener('click', function () {
          mobileNav.classList.remove('open');
        });
      });
    }

    // ── Active nav link ───────────────────────────────────────
    const currentPath = window.location.pathname.split('/').pop() || 'index.html';
    document.querySelectorAll('.nav-links a, .nav-mobile a').forEach(function (link) {
      const href = link.getAttribute('href');
      if (href && href.includes(currentPath)) {
        link.classList.add('active');
      }
    });

    // ── FAQ accordion ─────────────────────────────────────────
    document.querySelectorAll('.faq-question').forEach(function (btn) {
      btn.addEventListener('click', function () {
        const item = btn.closest('.faq-item');
        const wasOpen = item.classList.contains('open');
        // Close all
        document.querySelectorAll('.faq-item').forEach(function (el) {
          el.classList.remove('open');
        });
        if (!wasOpen) item.classList.add('open');
      });
    });

    // ── Scroll fade-in ────────────────────────────────────────
    const fadeEls = document.querySelectorAll('.fade-in');
    if (fadeEls.length) {
      const observer = new IntersectionObserver(
        function (entries) {
          entries.forEach(function (entry) {
            if (entry.isIntersecting) {
              entry.target.classList.add('visible');
              observer.unobserve(entry.target);
            }
          });
        },
        { threshold: 0.1, rootMargin: '0px 0px -40px 0px' }
      );
      fadeEls.forEach(function (el) { observer.observe(el); });
    }

    // ── Mermaid init ──────────────────────────────────────────
    if (typeof mermaid !== 'undefined') {
      const isDark = document.documentElement.getAttribute('data-theme') !== 'light';
      mermaid.initialize({
        startOnLoad: true,
        theme: isDark ? 'dark' : 'default',
        fontFamily: "'Inter', system-ui, sans-serif",
        fontSize: 14,
      });
    }
  });
})();
