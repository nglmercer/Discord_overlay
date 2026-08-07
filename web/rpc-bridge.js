(() => {
  const Native = window.WebSocket;
  const localSocket = /^ws:\/\/(?:127\.0\.0\.1|localhost):(\d+)\//i;

  // Published for /web/status.js. This is the only place that sees whether
  // Discord ever accepted a connection: the overlay probes ports 6463-6472 and
  // failures on the empty ones are normal, so only `opened` is meaningful.
  const rpc = (window.__discordOverlayRpc = { attempted: 0, opened: 0 });
  const announce = () => window.dispatchEvent(new CustomEvent("discord-overlay-rpc"));

  const Bridged = function (url, protocols) {
    let target = String(url);
    const match = localSocket.exec(target);
    // Never rewrite a socket that already points at this proxy.
    const bridged = match && match[1] !== location.port;
    if (bridged) {
      target = target.replace(localSocket, "ws://" + location.host + "/rpc/" + match[1] + "/");
    }

    const socket = protocols === undefined ? new Native(target) : new Native(target, protocols);
    if (bridged) {
      rpc.attempted += 1;
      announce();
      socket.addEventListener("open", () => {
        rpc.opened += 1;
        announce();
      });
    }
    return socket;
  };
  Bridged.prototype = Native.prototype;
  for (const key of ["CONNECTING", "OPEN", "CLOSING", "CLOSED"]) {
    Bridged[key] = Native[key];
  }
  window.WebSocket = Bridged;
})();
