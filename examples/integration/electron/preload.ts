import { contextBridge } from 'electron';

contextBridge.exposeInMainWorld('versions', {
  electron: process.versions.electron,
  node: process.versions.node,
  chrome: process.versions.chrome,
});
