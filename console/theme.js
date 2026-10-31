// Thème appliqué avant l'affichage, pour éviter un flash du mauvais thème.
try { const t = JSON.parse(localStorage.getItem("tonapi.theme")); if (t === "light" || t === "dark") document.documentElement.dataset.theme = t; } catch {}
