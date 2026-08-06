(() => {
  const Native = window.WebSocket;
  const localSocket = /^ws:\/\/(?:127\.0\.0\.1|localhost):(\d+)\//i;
  const Bridged = function (url, protocols) {
    let target = String(url);
    const match = localSocket.exec(target);
    // Never rewrite a socket that already points at this proxy.
    if (match && match[1] !== location.port) {
      target = target.replace(localSocket, "ws://" + location.host + "/rpc/" + match[1] + "/");
    }
    return protocols === undefined ? new Native(target) : new Native(target, protocols);
  };
  Bridged.prototype = Native.prototype;
  for (const key of ["CONNECTING", "OPEN", "CLOSING", "CLOSED"]) {
    Bridged[key] = Native[key];
  }
  window.WebSocket = Bridged;
})();
