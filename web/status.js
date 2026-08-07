(() => {
  /**
   * Grace period before the overlay is willing to say anything at all. A
   * healthy start connects well inside this window and never shows a message.
   */
  const GRACE_MS = 1500;

  const node = document.createElement("div");
  node.id = "discord-overlay-status-message";
  node.hidden = true;

  let graceOver = false;
  let settled = false;

  const rpc = () => window.__discordOverlayRpc ?? { attempted: 0, opened: 0 };

  /** Streamkit has rendered at least one voice state: everything works. */
  const live = () => document.querySelector("li.voice_state") !== null;

  /**
   * The RPC bridge is the only place that sees whether Discord ever accepted a
   * connection, which is what separates "Discord is closed" from "the overlay
   * never got far enough to try".
   */
  const diagnosis = () => {
    const { attempted, opened } = rpc();
    if (attempted === 0) return "Can’t reach Streamkit — its overlay script never started";
    if (opened === 0) return "Discord isn’t running — start the desktop app";
    return "Connected — waiting for someone in the voice channel";
  };

  const observer = new MutationObserver(() => update());

  /**
   * Stop for good once the overlay comes up. The message deliberately never
   * returns: someone leaving the channel mid-stream must not put text on it.
   */
  const stop = () => {
    settled = true;
    node.hidden = true;
    observer.disconnect();
    window.removeEventListener("discord-overlay-rpc", update);
  };

  function update() {
    if (settled) return;
    if (live()) {
      stop();
      return;
    }
    if (!graceOver) return;
    node.textContent = diagnosis();
    node.hidden = false;
  }

  document.body.appendChild(node);
  observer.observe(document.body, { childList: true, subtree: true });
  window.addEventListener("discord-overlay-rpc", update);
  setTimeout(() => {
    graceOver = true;
    update();
  }, GRACE_MS);
})();
