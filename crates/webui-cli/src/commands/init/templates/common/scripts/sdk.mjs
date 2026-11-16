// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { lstat, readFile, realpath } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const version = '__WEBUI_VERSION__';
const targets = {
  'darwin-arm64': ['aarch64-apple-darwin', 'libwebui_ffi.dylib'],
  'darwin-x64': ['x86_64-apple-darwin', 'libwebui_ffi.dylib'],
  'linux-arm64': ['aarch64-unknown-linux-gnu', 'libwebui_ffi.so'],
  'linux-x64': ['x86_64-unknown-linux-gnu', 'libwebui_ffi.so'],
  'win32-arm64': ['aarch64-pc-windows-msvc', 'webui_ffi.dll'],
  'win32-x64': ['x86_64-pc-windows-msvc', 'webui_ffi.dll'],
};

function platformArtifact() {
  const platform = `${process.platform}-${process.arch}`;
  const target = targets[platform];
  if (!target || (process.platform === 'linux'
      && !process.report?.getReport().header.glibcVersionRuntime)) {
    throw new Error(`WebUI SDK ${version} does not support ${platform} (Linux requires glibc).`);
  }
  return { platform, target: target[0], library: target[1] };
}

async function digest(file) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest('hex');
}

async function validateSdk(directory, artifact) {
  const root = await realpath(directory);
  const metadata = JSON.parse(await readFile(path.join(root, 'sdk.json'), 'utf8'));
  if (metadata.schemaVersion !== 1 || metadata.version !== version) {
    throw new Error(`WebUI SDK version mismatch: expected ${version}, received ${metadata.version ?? 'unknown'}.`);
  }
  if (metadata.target !== artifact.target || metadata.os !== process.platform
      || metadata.arch !== process.arch
      || metadata.libc !== (process.platform === 'linux' ? 'gnu' : null)) {
    throw new Error(`WebUI SDK platform mismatch: expected ${artifact.target}.`);
  }
  const files = ['include/webui_ffi.h', `lib/${artifact.library}`, 'LICENSE'];
  if (process.platform === 'win32') files.push('lib/webui_ffi.dll.lib');
  if (!metadata.files || typeof metadata.files !== 'object' || Array.isArray(metadata.files)
      || Object.keys(metadata.files).length !== files.length) {
    throw new Error('WebUI SDK metadata has an invalid file inventory.');
  }
  for (const relative of files) {
    const file = path.join(root, relative);
    const canonical = await realpath(file);
    if (!canonical.startsWith(`${root}${path.sep}`) || !(await lstat(file)).isFile()) {
      throw new Error(`WebUI SDK contains an unsafe file: ${relative}.`);
    }
    if (!/^[a-f0-9]{64}$/.test(metadata.files[relative] ?? '')
        || await digest(file) !== metadata.files[relative]) {
      throw new Error(`WebUI SDK integrity check failed for ${relative}; extract the matching standalone release archive again.`);
    }
  }
  return root;
}

/** Resolve only explicit or project-local standalone artifacts. Never downloads or builds an SDK. */
export async function provisionSdk(projectRoot = process.cwd()) {
  const artifact = platformArtifact();
  try {
    if (Object.hasOwn(process.env, 'WEBUI_SDK_DIR')) {
      if (!process.env.WEBUI_SDK_DIR) throw new Error('WEBUI_SDK_DIR must name a standalone SDK directory.');
      return await validateSdk(path.resolve(process.env.WEBUI_SDK_DIR), artifact);
    }
    return await validateSdk(path.resolve(projectRoot, '.webui/sdk'), artifact);
  } catch (error) {
    const archive = `webui-sdk-${version}-${artifact.platform}.tar.gz`;
    throw new Error(`Cannot provision WebUI SDK ${version}: ${error.message}\nDownload ${archive} and its SHA-256 file from https://github.com/microsoft/webui/releases/tag/v${version}, verify the checksum, and extract the SDK root into .webui/sdk or set WEBUI_SDK_DIR to its absolute directory. npm packages do not contain the SDK.`, { cause: error });
  }
}

/** Build and runtime environment for native hosts; pass it to spawn without a shell. */
export function sdkEnvironment(sdkRoot, baseEnv = process.env) {
  const root = path.resolve(sdkRoot);
  const lib = path.join(root, 'lib');
  const quote = (flag) => JSON.stringify(flag.replaceAll('\\', '/'));
  const loader = process.platform === 'win32'
    ? Object.keys(baseEnv).find((key) => key.toLowerCase() === 'path') ?? 'PATH'
    : process.platform === 'darwin' ? 'DYLD_LIBRARY_PATH' : 'LD_LIBRARY_PATH';
  return {
    ...baseEnv,
    WEBUI_SDK_DIR: root,
    CGO_ENABLED: '1',
    CGO_CPPFLAGS: [quote(`-I${path.join(root, 'include')}`), baseEnv.CGO_CPPFLAGS].filter(Boolean).join(' '),
    CGO_CFLAGS: [quote(`-I${path.join(root, 'include')}`), baseEnv.CGO_CFLAGS].filter(Boolean).join(' '),
    CGO_LDFLAGS: [
      quote(`-L${lib}`),
      baseEnv.CGO_LDFLAGS,
    ].filter(Boolean).join(' '),
    [loader]: [lib, baseEnv[loader]].filter(Boolean).join(path.delimiter),
  };
}

if (process.argv[1]
    && await realpath(process.argv[1]).catch(() => undefined) === fileURLToPath(import.meta.url)) {
  try {
    console.log(await provisionSdk());
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
