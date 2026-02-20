// This is a simple client-side script for the WebUI hello world example

document.addEventListener('DOMContentLoaded', function () {
    console.log('WebUI Hello World example loaded!');

    // Simple polling-based HMR: reload the page when the server version changes
    let currentVersion = null;

    async function checkVersion() {
        try {
            const res = await fetch('/hmr');
            if (!res.ok) {
                throw new Error('Failed to fetch HMR version');
            }
            const text = (await res.text()).trim();

            if (currentVersion === null) {
                currentVersion = text;
            } else if (currentVersion !== text) {
                window.location.reload();
                return;
            }
        } catch (e) {
            // Ignore errors and retry; server may be restarting
        } finally {
            setTimeout(checkVersion, 1000);
        }
    }

    checkVersion();
});
