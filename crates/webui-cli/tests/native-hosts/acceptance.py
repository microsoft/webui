# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

"""Build standalone native-host templates against an explicitly supplied SDK.

python3 .../tests/native-hosts/acceptance.py --sdk /absolute/sdk --dist /absolute/app/dist
Add --dotnet-feed /approved/local/feed to include the pinned NuGet consumer.
This intentionally never builds the SDK or enables NuGet publication.
"""

import argparse
import concurrent.futures
import http.client
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import tempfile
import time


def command(args, cwd, env):
    subprocess.run(args, cwd=cwd, env=env, check=True, timeout=180)


def request(port, path, method="GET"):
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
    try:
        connection.request(method, path)
        response = connection.getresponse()
        return response.status, dict((k.lower(), v) for k, v in response.getheaders()), response.read()
    finally:
        connection.close()


def verify(port):
    status, headers, home = request(port, "/")
    assert status == 200 and b"Hello WebUI" in home, (status, home)
    assert headers["content-type"] == "text/html; charset=utf-8"
    assert headers["x-content-type-options"] == "nosniff"
    assert b'/assets/index.js' in home
    status, _, about = request(port, "/about")
    assert status == 200 and b"About WebUI" in about, (status, about)
    assert b'/assets/index.js' not in about
    for path in ["/", "/about", "/assets/host-test.js", "/missing"]:
        status, headers, body = request(port, path)
        head_status, head_headers, head_body = request(port, path, "HEAD")
        assert status == head_status and not head_body, path
        assert headers["content-length"] == head_headers["content-length"] == str(len(body)), path
    status, headers, body = request(port, "/assets/host-test.js")
    assert status == 200 and body == b"export{}"
    assert headers["content-type"] == "text/javascript; charset=utf-8"
    for path in [
        "/missing", "/protocol.bin", "/dist/protocol.bin", "/server/main.c",
        "/assets/../protocol.bin", "/assets/%2e%2e/protocol.bin",
        "/assets/%2e%2e%2fprotocol.bin", "/assets/.secret.js", "/assets/secret-link.js",
        "/assets/source.ts", "/assets/folder-link/secret.js",
    ]:
        status, _, body = request(port, path)
        assert status == 404 and b"PRIVATE" not in body, (path, status)
    for method in ["POST", "PUT", "DELETE", "OPTIONS"]:
        status, headers, _ = request(port, "/", method)
        assert status == 405 and headers["allow"] == "GET, HEAD", method
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        responses = list(pool.map(lambda i: request(port, "/" if i % 2 == 0 else "/about"), range(64)))
    assert all(status == 200 for status, _, _ in responses)
    for i, (_, _, body) in enumerate(responses):
        assert (b'/assets/index.js' in body) == (i % 2 == 0), "request-local route state leaked"


def serve(binary, root, env):
    with socket.socket() as reservation:
        reservation.bind(("127.0.0.1", 0))
        port = reservation.getsockname()[1]
    env = dict(env, PORT=str(port), HOST="127.0.0.1", WEBUI_APP_ROOT=str(root))
    child = subprocess.Popen(binary, cwd=root, env=env)
    try:
        for _ in range(100):
            if child.poll() is not None:
                raise AssertionError(f"server exited before readiness: {child.returncode}")
            try:
                request(port, "/")
                break
            except OSError:
                time.sleep(0.1)
        else:
            raise AssertionError("server not ready after 10s")
        verify(port)
    finally:
        child.send_signal(signal.SIGTERM)
        try:
            code = child.wait(timeout=15)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait()
            raise AssertionError("server did not stop gracefully")
        assert code == 0, f"server exited with {code}"
    # Startup must reject a malformed protocol rather than serve empty HTML.
    protocol = root / "dist/protocol.bin"
    original = protocol.read_bytes()
    try:
        protocol.write_bytes(b"\xff")
        failed = subprocess.run(binary, cwd=root, env=env, timeout=10)
        assert failed.returncode != 0, "malformed protocol accepted"
    finally:
        protocol.write_bytes(original)
    public = root / "dist/public"
    backup = root / "dist/public-snapshot-test"
    public.rename(backup)
    try:
        public.symlink_to(root, target_is_directory=True)
        failed = subprocess.run(binary, cwd=root, env=env, timeout=10)
        assert failed.returncode != 0, "symlink public root accepted"
    finally:
        public.unlink()
        backup.rename(public)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sdk", type=Path, required=True)
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument("--cli", type=Path, default=Path("target/debug/webui"))
    parser.add_argument("--dotnet-feed", type=Path)
    parser.add_argument("--hosts", nargs="+", default=["c", "cpp", "go"])
    args = parser.parse_args()
    sdk, dist, cli = args.sdk.resolve(), args.dist.resolve(), args.cli.resolve()
    env = dict(os.environ, WEBUI_SDK_DIR=str(sdk), CGO_ENABLED="1")
    env["CGO_CFLAGS"] = f'"-I{sdk / "include"}"'
    env["CGO_LDFLAGS"] = f'"-L{sdk / "lib"}"'
    loader = "DYLD_LIBRARY_PATH" if os.uname().sysname == "Darwin" else "LD_LIBRARY_PATH"
    env[loader] = str(sdk / "lib") + os.pathsep + env.get(loader, "")
    with tempfile.TemporaryDirectory(prefix="webui native hosts ") as temporary:
        # Keep release package restoration isolated from the user's cache.
        env["NUGET_PACKAGES"] = str(Path(temporary) / "nuget")
        for host in args.hosts:
            root = Path(temporary).resolve() / host
            server = root / "server"
            command([str(cli), "init", str(root), "--server", host, "--no-interactive"], Path(temporary), env)
            shutil.copytree(dist, root / "dist")
            public = root / "dist/public"
            (public / "host-test.js").write_bytes(b"export{}")
            (public / ".secret.js").write_bytes(b"PRIVATE")
            (public / "source.ts").write_bytes(b"PRIVATE")
            (root / "secret.js").write_bytes(b"PRIVATE")
            (public / "secret-link.js").symlink_to(root / "secret.js")
            (public / "folder-link").symlink_to(root, target_is_directory=True)
            if host in ["c", "cpp"]:
                command(["cmake", "-S", "server", "-B", "server/build", "-DCMAKE_BUILD_TYPE=Release"], root, env)
                command(["cmake", "--build", "server/build", "--parallel", "2"], root, env)
                binary = [str(server / "build/webui-app")]
                if os.uname().sysname == "Darwin":
                    links = subprocess.check_output(["otool", "-L", binary[0]], text=True)
                    assert "@rpath/libwebui_ffi.dylib" in links, "SDK dylib has a nonrelocatable install name"
            elif host == "go":
                command(["go", "mod", "download", "all"], server, env)
                command(["go", "test", "-race", "./..."], server, env)
                command(["go", "build", "-o", "build/webui-app", "."], server, env)
                binary = [str(server / "build/webui-app")]
            elif host == "dotnet":
                if args.dotnet_feed is None:
                    parser.error("--dotnet-feed is required to test the .NET package consumer")
                command(["dotnet", "restore", "server/App.csproj", "--source", str(args.dotnet_feed.resolve())], root, env)
                command(["dotnet", "build", "server/App.csproj", "-c", "Release", "--no-restore"], root, env)
                binary = [shutil.which("dotnet"), str(server / "bin/Release/net8.0/App.dll")]
            else:
                parser.error(f"unknown host {host}")
            runtime_env = env.copy()
            # Deployed rendering must not discover Node, npm, or build tools.
            runtime_env["PATH"] = ""
            if host in ["c", "cpp"]:
                # Exercise CMake's runtime search path, not an inherited loader override.
                runtime_env.pop(loader, None)
            if host == "dotnet":
                runtime_env.pop(loader, None)
                runtime_env.pop("WEBUI_LIB_PATH", None)
                runtime_env["WEBUI_SDK_DIR"] = str(root / "intentionally-missing-sdk")
            serve(binary, root, runtime_env)
            print(f"PASS {host}: startup, SSR, HEAD, static containment/MIME, 404/405, concurrency, malformed protocol, symlink root, shutdown")


if __name__ == "__main__":
    main()
