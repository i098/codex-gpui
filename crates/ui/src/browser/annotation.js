(() => {
  let generation = 0;
  let mode = 0;
  let space = false;
  let down;
  let outline;
  let suppressClick = false;

  function clear() {
    down = undefined;
    outline?.remove();
    outline = undefined;
  }

  globalThis.codexAnnotation = (id, nextMode) => {
    if (id !== generation || nextMode === 0) {
      clear();
      space = false;
    }
    generation = id;
    mode = nextMode;
  };

  for (const active of [true, false]) {
    addEventListener(`codex:annotation-mode-${active ? 'on' : 'off'}`, (event) => {
      if (event.target !== window || !isSecureContext ||
          !navigator.userActivation?.isActive || mode > 1) return;
      event.preventDefault();
      window.webkit.messageHandlers.codexAnnotation.postMessage(JSON.stringify({
        siteRequest: { generation, active },
      }));
    }, true);
  }

  function block(event) {
    event.preventDefault();
    event.stopImmediatePropagation();
  }

  function bind(name, listener) {
    addEventListener(name, (event) => {
      if (event.isTrusted) listener(event);
    }, true);
  }

  function rect(left, top, right, bottom) {
    const x = Math.max(0, left);
    const y = Math.max(0, top);
    return { x, y, width: Math.min(innerWidth, right) - x,
      height: Math.min(innerHeight, bottom) - y };
  }

  function bounds({left, top, right, bottom}) {
    return rect(left, top, right, bottom);
  }

  function area(event) {
    const {clientX: x, clientY: y} = event;
    return rect(Math.min(x, down.x), Math.min(y, down.y),
      Math.max(x, down.x), Math.max(y, down.y));
  }

  function paint(box, cursor) {
    if (!outline?.isConnected) {
      outline = document.createElement('div');
      outline.setAttribute('aria-hidden', 'true');
      document.documentElement.append(outline);
    }
    Object.assign(outline.style, {
      all: 'initial', position: 'fixed', zIndex: '2147483647', pointerEvents: 'none',
      boxSizing: 'border-box', border: '2px solid #487ff5', borderRadius: '3px',
      left: `${box.x}px`, top: `${box.y}px`, width: `${box.width}px`, height: `${box.height}px`,
      backgroundColor: cursor ? '#487ff51a' : '#487ff533',
    });
  }

  function report(kind, targetRect, details = {}) {
    if (targetRect.width <= 0 || targetRect.height <= 0) return;
    window.webkit.messageHandlers.codexAnnotation.postMessage(JSON.stringify({
      generation, kind, targetRect, ...details,
    }));
  }

  bind('keydown', (event) => {
    if (!mode || event.code !== 'Space' || event.altKey || event.metaKey ||
        event.ctrlKey || event.shiftKey || event.isComposing) return;
    space = true;
    clear();
    block(event);
  });
  bind('keyup', (event) => {
    if (event.code !== 'Space' || !space) return;
    space = false;
    block(event);
  });
  bind('blur', (event) => {
    if (event.target === window) {
      space = false;
      clear();
    }
  });
  bind('scroll', clear);
  bind('pointercancel', clear);
  bind('pointerdown', (event) => {
    suppressClick = false;
    if (!mode || space || !event.isPrimary || event.button !== 0) return;
    if (mode === 3 || (mode === 2 && !event.shiftKey)) {
      suppressClick = true;
      block(event);
      return;
    }
    if (event.altKey && mode === 1) {
      getSelection()?.removeAllRanges();
      clear();
      down = { id: event.pointerId, text: true };
      return;
    }
    down = { id: event.pointerId, x: event.clientX, y: event.clientY };
    suppressClick = true;
    block(event);
  });
  bind('pointermove', (event) => {
    if (!mode || space || mode === 3 || (mode === 2 && !event.shiftKey) || event.altKey || down?.text) {
      if (!down) clear();
      return;
    }
    const target = document.elementFromPoint(event.clientX, event.clientY);
    if (down && down.id === event.pointerId) paint(area(event), false);
    else if (target) paint(bounds(target.getBoundingClientRect()), true);
  });
  bind('pointerup', (event) => {
    if (suppressClick) setTimeout(() => { suppressClick = false; }, 0);
    if (down?.id !== event.pointerId) return;
    if (down.text) {
      const selection = getSelection();
      const text = selection?.toString().trim().slice(0, 4000);
      if (text && selection.rangeCount) {
        report('text', bounds(selection.getRangeAt(0).getBoundingClientRect()), { selectedText: text });
      }
    } else {
      block(event);
      const box = area(event);
      if (Math.hypot(box.width, box.height) >= 5 && box.width > 0 && box.height > 0) {
        report('region', box);
      } else {
        const target = document.elementFromPoint(event.clientX, event.clientY);
        if (target) {
          const tag = target.localName;
          const selector = target.id ? `#${CSS.escape(target.id)}` : tag;
          report('element', bounds(target.getBoundingClientRect()), {
            targetSelector: selector.slice(0, 512),
            targetRole: target.getAttribute('role')?.slice(0, 128),
            targetDescription: (target.getAttribute('aria-label') ||
              target.textContent.slice(0, 1024).trim() || tag).slice(0, 512),
          });
        }
      }
    }
    clear();
  });
  for (const name of ['click', 'auxclick', 'dblclick']) {
    bind(name, (event) => {
      if (mode && suppressClick && event.detail > 0) block(event);
    });
  }
})();
