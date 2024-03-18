export interface BTRPlugin {
  onCopyEnd(): CopyEndPlugin
}

export interface CopyEndPlugin {
  addFile(file: string): void
  process(): void
}