(() => {
  if (!isSecureContext) return;
  const settingsEvent = 'browser-sidebar-site-annotation-api-enabled-changed';
  const requestEvent = 'codex:annotation-mode-';
  let enabled = false;
  let active = false;
  let installed = false;
  addEventListener(settingsEvent, ({ detail }) => {
    enabled = detail?.enabled === true;
    active = detail?.active === true;
    if (enabled && !installed) installed = install();
  });

  function install() {
    const oai = document.oai ?? {};
    if ((typeof oai !== 'object' && typeof oai !== 'function') || oai.annotation != null) return false;
    const annotation = {
      toggle(force) {
        if (!enabled || !navigator.userActivation?.isActive ||
            (force !== undefined && typeof force !== 'boolean')) return { accepted: false };
        const event = new Event(requestEvent + ((force ?? !active) ? 'on' : 'off'), { cancelable: true });
        return { accepted: !dispatchEvent(event) };
      },
      isActive: () => active,
    };
    return Reflect.defineProperty(oai, 'annotation', { configurable: true, value: annotation }) &&
      (document.oai != null || Reflect.defineProperty(document, 'oai', { configurable: true, value: oai }));
  }
})();
