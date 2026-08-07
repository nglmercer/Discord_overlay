(() => {
  const currentScript = document.currentScript;
  const scriptUrl = currentScript instanceof HTMLScriptElement
    ? currentScript.src
    : location.href;
  const version = Number(new URL(scriptUrl, location.href).searchParams.get("version") ?? "0");
  const endpoint = new URL("/reload-events", location.origin).href;
  const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

  (async () => {
    for (;;) {
      try {
        const res = await fetch(endpoint + "?since=" + version, { cache: "no-store" });
        const next = Number(await res.text());
        if (!res.ok || !Number.isFinite(next)) {
          // An error page answers instantly; without a pause this would spin.
          await sleep(2000);
          continue;
        }
        if (next !== version) {
          location.reload();
          return;
        }
      } catch (err) {
        // Proxy restarting or unreachable — back off and keep trying.
        await sleep(2000);
      }
    }
  })();
})();
