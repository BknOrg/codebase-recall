// render: the requestAnimationFrame draw loop — edges, nodes, labels,
// arrows — with viewport culling and level-of-detail.

  let drawPending = false;
  function scheduleDraw() {
    if (drawPending) return;
    drawPending = true;
    requestAnimationFrame(() => {
      drawPending = false;
      draw();
    });
  }

  function draw() {
    const k = transform.k;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    ctx.save();
    ctx.translate(transform.x, transform.y);
    ctx.scale(k, k);

    const pad = 80;
    const vx0 = -transform.x / k - pad;
    const vy0 = -transform.y / k - pad;
    const vx1 = (width - transform.x) / k + pad;
    const vy1 = (height - transform.y) / k + pad;

    const hs = highlightSet();

    for (const l of links) {
      const s = l.source;
      const t = l.target;
      if (!s || !t || s.x == null) continue;
      if (
        Math.max(s.x, t.x) < vx0 ||
        Math.min(s.x, t.x) > vx1 ||
        Math.max(s.y, t.y) < vy0 ||
        Math.min(s.y, t.y) > vy1
      )
        continue;

      const near = hovered && (s.id === hovered || t.id === hovered);
      const on = hs ? hs.has(s.id) && hs.has(t.id) : true;

      if (l.kind === "contains") {
        ctx.globalAlpha = near ? 0.5 : on ? 0.16 : 0.05;
        ctx.strokeStyle = COL.edge;
        ctx.lineWidth = 0.8 / k;
        ctx.beginPath();
        ctx.moveTo(s.x, s.y);
        ctx.lineTo(t.x, t.y);
        ctx.stroke();
        continue;
      }

      if (k < 0.22 && l.kind !== "imports") continue;
      ctx.globalAlpha = near ? 1 : on ? 0.4 : 0.08;
      ctx.strokeStyle = near ? COL.accent : COL.edge;
      ctx.lineWidth = (near ? 2 : Math.min(3, 1 + Math.log2(1 + l.count))) / k;
      ctx.beginPath();
      ctx.moveTo(s.x, s.y);
      ctx.lineTo(t.x, t.y);
      ctx.stroke();
      if (k > 0.4) drawArrow(s, t, near, on);
    }
    ctx.globalAlpha = 1;

    const showLabels = k > 0.55;
    for (const d of nodes) {
      if (d.x == null || d.x < vx0 || d.x > vx1 || d.y < vy0 || d.y > vy1) continue;
      const r = nodeRadius(d);
      const dim = hs && !hs.has(d.id) && d.id !== hovered;
      ctx.globalAlpha = dim ? 0.16 : 1;
      ctx.fillStyle = kindColor(d.kind);

      if (d.kind === "dir") {
        roundRect(d.x - r, d.y - r, r * 2, r * 2, 3 / k + 1);
        ctx.fill();
      } else if (k < 0.16) {
        ctx.fillRect(d.x - r * 0.8, d.y - r * 0.8, r * 1.6, r * 1.6);
        ctx.globalAlpha = 1;
        continue;
      } else {
        ctx.beginPath();
        ctx.arc(d.x, d.y, r, 0, 2 * Math.PI);
        ctx.fill();
      }

      let sw = 1.5 / k;
      let sc = COL.bg;
      if (d.id === hovered) {
        sc = COL.accent;
        sw = 3 / k;
      } else if (d.id === selected) {
        sc = COL.accent;
        sw = 2.5 / k;
      } else if (hs && hs.has(d.id)) {
        sc = COL.fg;
        sw = 1.8 / k;
      } else if (state.query && hit(d, state.query)) {
        sc = COL.accent;
        sw = 2 / k;
      }
      ctx.lineWidth = sw;
      ctx.strokeStyle = sc;
      ctx.stroke();

      const canExpand =
        (d.kind === "dir") || (d.kind === "file" && childrenOf.has(d.id) && !expanded.has(d.id));
      if (canExpand && k > 0.3) {
        ctx.fillStyle = d.kind === "dir" ? COL.bg : COL.fg;
        ctx.font = `bold ${(d.kind === "dir" ? r * 1.4 : 8) / (d.kind === "dir" ? 1 : k)}px system-ui, sans-serif`;
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        if (d.kind === "dir") ctx.fillText("+", d.x, d.y + 0.5 / k);
        else {
          ctx.fillStyle = COL.bg;
          ctx.beginPath();
          ctx.arc(d.x + r * 0.9, d.y - r * 0.9, 2.4 / k, 0, 2 * Math.PI);
          ctx.fill();
        }
        ctx.textAlign = "left";
      }

      const bigEnough = r * k > 5;
      if (
        (showLabels && bigEnough) ||
        d.id === hovered ||
        d.id === selected ||
        (state.query && hit(d, state.query))
      )
        drawLabel(d, r, k, dim);
      ctx.globalAlpha = 1;
    }

    ctx.restore();
  }

  function drawLabel(d, r, k, dim) {
    let txt =
      d.kind === "dir" || d.kind === "file" ? d.label.split("/").pop() || d.label : d.label;
    if (txt.length > 42) txt = txt.slice(0, 41) + "…";
    ctx.font = `${11 / k}px system-ui, -apple-system, Segoe UI, Roboto, sans-serif`;
    ctx.textBaseline = "middle";
    ctx.globalAlpha = dim ? 0.12 : 1;
    ctx.lineWidth = 3.5 / k;
    ctx.strokeStyle = COL.bg;
    ctx.lineJoin = "round";
    const x = d.x + r + 4 / k;
    ctx.strokeText(txt, x, d.y);
    ctx.fillStyle = COL.fg;
    ctx.fillText(txt, x, d.y);
  }

  function drawArrow(s, t, near, on) {
    const dx = t.x - s.x;
    const dy = t.y - s.y;
    const len = Math.hypot(dx, dy) || 1;
    const ux = dx / len;
    const uy = dy / len;
    const k = transform.k;
    const tr = nodeRadius(t) + 1.5;
    const bx = t.x - ux * tr;
    const by = t.y - uy * tr;
    const a = 5 / k;
    ctx.globalAlpha = near ? 1 : on ? 0.5 : 0.1;
    ctx.fillStyle = near ? COL.accent : COL.edge;
    ctx.beginPath();
    ctx.moveTo(bx, by);
    ctx.lineTo(bx - ux * a - uy * a * 0.55, by - uy * a + ux * a * 0.55);
    ctx.lineTo(bx - ux * a + uy * a * 0.55, by - uy * a - ux * a * 0.55);
    ctx.closePath();
    ctx.fill();
  }

  function roundRect(x, y, w, h, rad) {
    rad = Math.min(rad, w / 2, h / 2);
    ctx.beginPath();
    ctx.moveTo(x + rad, y);
    ctx.arcTo(x + w, y, x + w, y + h, rad);
    ctx.arcTo(x + w, y + h, x, y + h, rad);
    ctx.arcTo(x, y + h, x, y, rad);
    ctx.arcTo(x, y, x + w, y, rad);
    ctx.closePath();
  }
