// Stands in for the Discord client an activity expects to be framed by. The
// Embedded App SDK posts its RPC to `window.parent` and takes replies as
// `message` events; loaded top-level, the parent is the page itself, so it is
// replaced with one that hands messages to the app, and replies are fired as
// events from an origin the SDK accepts.
(function () {
    if (window.top !== window.self || !location.hostname.endsWith(".discordsays.com")) {
        return;
    }
    const host = {
        opener: null,
        postMessage(message) {
            try {
                window.ipc.postMessage(JSON.stringify(message));
            } catch (error) {
                // A message that can't be serialised has nothing the app could answer.
            }
        },
    };
    try {
        Object.defineProperty(window, "parent", { get: () => host, configurable: true });
    } catch (error) {
        window.parent = host;
    }
    Object.defineProperty(window, "__oxidecordReceive", {
        value(message) {
            window.dispatchEvent(
                new MessageEvent("message", { data: message, origin: "https://discord.com" }),
            );
        },
    });
})();
