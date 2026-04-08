<script setup>
import { ref, reactive, watch, onMounted, onUnmounted, nextTick, shallowRef } from 'vue'

// --- Virtual file system ---
const files = reactive({
  'index.html': `<h1>Hello, {{name}}!</h1>
<p>Welcome to the WebUI Playground.</p>

<if condition="showGreeting">
  <p>{{greeting}}</p>
</if>

<h2>Team</h2>
<for each="person in people">
  <person-card>{{person.name}} - {{person.role}}</person-card>
</for>`,
  'person-card.html': `<div class="card">
  <slot></slot>
</div>`,
  'person-card.css': `.card {
  padding: 8px 16px;
  margin: 4px 0;
  border-left: 3px solid #646cff;
}`,
  'state.json': JSON.stringify({
    name: "WebUI",
    greeting: "This framework rocks!",
    showGreeting: true,
    people: [
      { name: "Alice", role: "Engineer" },
      { name: "Bob", role: "Designer" },
      { name: "Charlie", role: "PM" }
    ]
  }, null, 2)
})

const activeFile = ref('index.html')
const previewHtml = ref('')
const errorMsg = ref('')
const wasmReady = ref(false)
const wasmModule = shallowRef(null)
const buildTime = ref(null)
const renderTime = ref(null)
let themeObserver = null
let isDarkTheme = false

// --- File operations ---
const newFileName = ref('')
const showNewFileInput = ref(false)
const newFileInput = ref(null)
const mobileSidebarOpen = ref(false)

function isMobileViewport() {
  return typeof window !== 'undefined' && window.matchMedia('(max-width: 768px)').matches
}

function toggleMobileSidebar() {
  mobileSidebarOpen.value = !mobileSidebarOpen.value
}

function closeMobileSidebar() {
  mobileSidebarOpen.value = false
}

function selectFile(name) {
  activeFile.value = name
  if (isMobileViewport()) {
    mobileSidebarOpen.value = false
  }
}

function openNewFileInput() {
  showNewFileInput.value = true
  nextTick(() => {
    if (newFileInput.value) {
      newFileInput.value.focus()
      newFileInput.value.select()
    }
  })
}

function addFile() {
  const name = newFileName.value.trim()
  if (name && !files[name]) {
    files[name] = ''
    activeFile.value = name
  }
  newFileName.value = ''
  showNewFileInput.value = false
}

function deleteFile(name) {
  if (name === 'index.html' || name === 'state.json') return
  delete files[name]
  if (activeFile.value === name) {
    activeFile.value = 'index.html'
  }
}

// --- CodeMirror setup ---
const editorContainer = ref(null)
let editorView = null

function getLanguage(filename) {
  if (filename.endsWith('.css')) return 'css'
  if (filename.endsWith('.json')) return 'json'
  return 'html'
}

function getFileIcon(filename) {
  if (filename.endsWith('.css')) return '●'
  if (filename.endsWith('.json')) return '◆'
  return '◇'
}

function getFileIconColor(filename) {
  if (filename.endsWith('.css')) return 'var(--vp-c-brand-2)'
  if (filename.endsWith('.json')) return 'var(--vp-c-warning-1)'
  return 'var(--vp-c-brand-1)'
}

function readThemeVar(name, fallback) {
  const value = getComputedStyle(document.documentElement)
    .getPropertyValue(name)
    .trim()
  return value || fallback
}

async function setupEditor() {
  if (!editorContainer.value) return

  const { EditorView, keymap, lineNumbers, highlightActiveLine, highlightSpecialChars } = await import('@codemirror/view')
  const { EditorState } = await import('@codemirror/state')
  const { defaultKeymap, history, historyKeymap } = await import('@codemirror/commands')
  const { oneDark } = await import('@codemirror/theme-one-dark')
  const { bracketMatching, syntaxHighlighting, defaultHighlightStyle } = await import('@codemirror/language')

  const lang = getLanguage(activeFile.value)
  let langExt
  if (lang === 'css') {
    const { css } = await import('@codemirror/lang-css')
    langExt = css()
  } else if (lang === 'json') {
    const { json } = await import('@codemirror/lang-json')
    langExt = json()
  } else {
    const { html } = await import('@codemirror/lang-html')
    langExt = html()
  }

  if (editorView) {
    editorView.destroy()
  }

  const updateListener = EditorView.updateListener.of((update) => {
    if (update.docChanged) {
      files[activeFile.value] = update.state.doc.toString()
      debouncedRender()
    }
  })

  const isDark = document.documentElement.classList.contains('dark')
  const editorTheme = EditorView.theme({
    '&': {
      height: '100%',
      fontSize: '13px',
      backgroundColor: 'var(--vp-c-bg-soft)',
      color: 'var(--vp-c-text-1)',
    },
    '.cm-scroller': { overflow: 'auto' },
    '.cm-gutters': {
      border: 'none',
      backgroundColor: 'var(--vp-c-bg-mute)',
      color: 'var(--vp-c-text-3)',
    },
    '.cm-content': {
      fontFamily: 'var(--vp-font-family-mono)',
    },
    '.cm-line': { padding: '0 8px' },
    '.cm-activeLine': {
      backgroundColor: 'var(--vp-c-default-soft)',
    },
    '.cm-activeLineGutter': {
      backgroundColor: 'var(--vp-c-default-soft)',
    },
    '.cm-selectionBackground, &.cm-focused .cm-selectionBackground, ::selection': {
      backgroundColor: 'var(--vp-c-brand-soft)',
    },
    '.cm-cursor, .cm-dropCursor': {
      borderLeftColor: 'var(--vp-c-brand-1)',
    },
    '.cm-focused': {
      outline: 'none',
    },
  })

  editorView = new EditorView({
    state: EditorState.create({
      doc: files[activeFile.value] || '',
      extensions: [
        lineNumbers(),
        highlightActiveLine(),
        highlightSpecialChars(),
        history(),
        bracketMatching(),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        langExt,
        ...(isDark ? [oneDark] : [syntaxHighlighting(defaultHighlightStyle, { fallback: true })]),
        updateListener,
        editorTheme,
      ],
    }),
    parent: editorContainer.value,
  })
}

// --- WASM rendering ---
let renderTimeout = null
function debouncedRender() {
  if (renderTimeout) clearTimeout(renderTimeout)
  renderTimeout = setTimeout(render, 150)
}

async function render() {
  if (!wasmModule.value) {
    errorMsg.value = 'WASM module not loaded yet'
    return
  }

  try {
    errorMsg.value = ''
    const filesObj = {}
    for (const [name, content] of Object.entries(files)) {
      if (name !== 'state.json') {
        filesObj[name] = content
      }
    }

    const stateJson = files['state.json'] || '{}'

    // Time the build (parse → protocol) step
    const t0 = performance.now()
    const protocolJson = wasmModule.value.build_protocol(filesObj, 'index.html')
    const t1 = performance.now()
    buildTime.value = (t1 - t0).toFixed(1)

    // Time the render (protocol + state → HTML) step
    const t2 = performance.now()
    const html = wasmModule.value.render(protocolJson, stateJson, 'index.html', '/')
    const t3 = performance.now()
    renderTime.value = (t3 - t2).toFixed(1)

    // Collect CSS from component files
    let css = ''
    for (const [name, content] of Object.entries(files)) {
      if (name.endsWith('.css') && name !== 'state.json') {
        css += content + '\n'
      }
    }

    const bodyBg = readThemeVar('--vp-c-bg', '#ffffff')
    const bodyText = readThemeVar('--vp-c-text-1', '#213547')
    const border = readThemeVar('--vp-c-divider', '#e2e2e3')
    const mono = readThemeVar('--vp-font-family-mono', "'SFMono-Regular', Consolas, 'Liberation Mono', Menlo, monospace")
    const base = readThemeVar('--vp-font-family-base', "Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif")

    previewHtml.value = `<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="color-scheme" content="light dark">
  <style>
    *, *::before, *::after { box-sizing: border-box; }
    body {
      font-family: ${base};
      padding: 24px;
      margin: 0;
      color: ${bodyText};
      background: ${bodyBg};
      line-height: 1.6;
    }
    h1, h2, h3, h4, h5, h6 {
      color: ${bodyText};
      margin-top: 0;
    }
    code, pre {
      font-family: ${mono};
    }
    hr {
      border: 0;
      border-top: 1px solid ${border};
    }
    ${css}
  </style>
</head>
<body>${html}</body>
</html>`
  } catch (e) {
    errorMsg.value = String(e)
    previewHtml.value = ''
  }
}

// --- Load WASM ---
async function loadWasm() {
  try {
    const base = import.meta.env.BASE_URL || '/'
    const wasmUrl = new URL(`${base}wasm/webui_wasm.js`, window.location.origin).href
    const mod = await import(/* @vite-ignore */ wasmUrl)
    await mod.default()
    wasmModule.value = mod
    wasmReady.value = true
    render()
  } catch (e) {
    errorMsg.value = 'Failed to load WASM module: ' + String(e)
  }
}

function handleThemeChange() {
  const nextTheme = document.documentElement.classList.contains('dark')
  if (nextTheme === isDarkTheme) return
  isDarkTheme = nextTheme

  nextTick(() => {
    setupEditor()
    if (wasmReady.value) {
      render()
    }
  })
}

// --- Lifecycle ---
watch(activeFile, () => {
  nextTick(setupEditor)
})

onMounted(async () => {
  document.documentElement.style.overflow = 'hidden'
  document.documentElement.classList.add('playground-active')
  isDarkTheme = document.documentElement.classList.contains('dark')

  themeObserver = new MutationObserver((mutations) => {
    for (const mutation of mutations) {
      if (mutation.type === 'attributes' && mutation.attributeName === 'class') {
        handleThemeChange()
        break
      }
    }
  })
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['class'],
  })

  await loadWasm()
  await nextTick()
  setupEditor()
})

onUnmounted(() => {
  document.documentElement.style.overflow = ''
  document.documentElement.classList.remove('playground-active')
  if (themeObserver) {
    themeObserver.disconnect()
    themeObserver = null
  }
})
</script>

<template>
  <div class="playground-shell">
    <!-- Main content -->
    <div class="playground-main">
      <!-- Editor area -->
      <div class="editor-area">
        <!-- Editor tabs -->
        <div class="tab-bar">
          <div
            v-for="(_, name) in files"
            :key="name"
            class="tab"
            :class="{ active: activeFile === name }"
            @click="activeFile = name"
          >
            <span class="tab-icon" :style="{ color: getFileIconColor(name) }">{{ getFileIcon(name) }}</span>
            <span class="tab-name">{{ name }}</span>
            <button
              v-if="name !== 'index.html' && name !== 'state.json'"
              class="tab-close-btn"
              @click.stop="deleteFile(name)"
              title="Close file"
            >
              <svg viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" stroke-width="2"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
            </button>
          </div>
          <button class="tab-add-btn" @click="openNewFileInput" title="New file">
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
          </button>
        </div>
        <div v-if="showNewFileInput" class="tab-new-file-row">
          <input
            ref="newFileInput"
            v-model="newFileName"
            @keyup.enter="addFile"
            @keyup.escape="showNewFileInput = false"
            @blur="showNewFileInput = false"
            placeholder="filename.html"
            autofocus
          />
        </div>
        <div ref="editorContainer" class="editor-container"></div>
      </div>

      <!-- Divider -->
      <div class="panel-divider"></div>

      <!-- Preview area -->
      <div class="preview-area">
        <div class="preview-header">
          <div class="preview-header-left">
            <span class="preview-title">Preview</span>
            <span class="preview-badge live">Live</span>
          </div>
          <div class="preview-stats" v-if="buildTime !== null">
            <span class="stat-badge build">Build {{ buildTime }}ms</span>
            <span class="stat-badge render">Render {{ renderTime }}ms</span>
          </div>
        </div>
        <div v-if="errorMsg" class="error-bar">
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/></svg>
          <span>{{ errorMsg }}</span>
        </div>
        <iframe
          v-if="previewHtml"
          :srcdoc="previewHtml"
          class="preview-frame"
          sandbox="allow-scripts"
        ></iframe>
        <div v-else-if="!errorMsg" class="preview-empty">
          <div class="empty-icon">⚡</div>
          <p>Preview will appear here</p>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.playground-shell {
  position: fixed;
  top: var(--vp-nav-height, 64px);
  left: 0;
  right: 0;
  bottom: 0;
  display: flex;
  flex-direction: column;
  background: var(--vp-c-bg);
  color: var(--vp-c-text-1);
  overflow: hidden;
  z-index: 10;
}

.mobile-files-btn,
.mobile-overlay {
  display: none;
}

/* ─── Main layout ─── */
.playground-main {
  display: flex;
  flex: 1;
  min-height: 0;
}

/* ─── Sidebar ─── */
.sidebar {
  width: 200px;
  flex-shrink: 0;
  background: var(--vp-c-bg-soft);
  border-right: 1px solid var(--vp-c-divider);
  display: flex;
  flex-direction: column;
}

.sidebar-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.1em;
  color: var(--vp-c-text-3);
}

.sidebar-btn {
  background: none;
  border: none;
  color: var(--vp-c-text-3);
  cursor: pointer;
  padding: 2px;
  border-radius: 3px;
  display: flex;
  align-items: center;
}
.sidebar-btn:hover { color: var(--vp-c-text-1); background: var(--vp-c-default-soft); }

.new-file-row {
  padding: 2px 8px 6px;
}
.new-file-row input {
  width: 100%;
  padding: 4px 8px;
  font-size: 12px;
  border: 1px solid var(--vp-c-brand-1);
  border-radius: 4px;
  background: var(--vp-c-bg);
  color: var(--vp-c-text-1);
  outline: none;
  font-family: inherit;
}

.file-list {
  flex: 1;
  overflow-y: auto;
}

.file-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 14px;
  cursor: pointer;
  font-size: 13px;
  color: var(--vp-c-text-2);
  transition: all 0.12s;
  border-left: 2px solid transparent;
}
.file-item:hover {
  background: var(--vp-c-default-soft);
  color: var(--vp-c-text-1);
}
.file-item.active {
  background: var(--vp-c-brand-soft);
  color: var(--vp-c-text-1);
  border-left-color: var(--vp-c-brand-1);
}

.file-icon { font-size: 10px; flex-shrink: 0; }
.file-name {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.delete-btn {
  background: none;
  border: none;
  color: var(--vp-c-text-3);
  cursor: pointer;
  padding: 2px;
  border-radius: 3px;
  display: flex;
  align-items: center;
  opacity: 0;
  transition: opacity 0.12s;
}
.file-item:hover .delete-btn { opacity: 1; }
.delete-btn:hover { color: var(--vp-c-danger-1); background: var(--vp-c-default-soft); }

/* ─── Editor area ─── */
/* ─── Editor area ─── */
.editor-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  background: var(--vp-c-bg);
}

.tab-bar {
  display: flex;
  background: var(--vp-c-bg-soft);
  border-bottom: 1px solid var(--vp-c-divider);
  overflow-x: auto;
  flex-shrink: 0;
}
.tab-bar::-webkit-scrollbar { height: 0; }

.tab {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 16px;
  font-size: 12px;
  color: var(--vp-c-text-2);
  cursor: pointer;
  border-right: 1px solid var(--vp-c-divider);
  white-space: nowrap;
  transition: all 0.12s;
  border-bottom: 2px solid transparent;
}
.tab:hover {
  color: var(--vp-c-text-1);
  background: var(--vp-c-default-soft);
}
.tab.active {
  color: var(--vp-c-text-1);
  background: var(--vp-c-bg);
  border-bottom-color: var(--vp-c-brand-1);
}

.tab-icon { font-size: 9px; }
.tab-name { font-family: inherit; }

.tab-close-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border: none;
  background: transparent;
  color: var(--vp-c-text-3);
  cursor: pointer;
  border-radius: 4px;
  padding: 0;
  margin-left: 2px;
  opacity: 0;
  transition: all 0.12s;
}
.tab:hover .tab-close-btn { opacity: 1; }
.tab-close-btn:hover { color: var(--vp-c-text-1); background: var(--vp-c-default-soft); }

.tab-add-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  min-width: 32px;
  border: none;
  background: transparent;
  color: var(--vp-c-text-3);
  cursor: pointer;
  transition: all 0.12s;
}
.tab-add-btn:hover { color: var(--vp-c-text-1); background: var(--vp-c-default-soft); }

.tab-new-file-row {
  padding: 4px 8px;
  border-bottom: 1px solid var(--vp-c-divider);
  background: var(--vp-c-bg-alt);
}
.tab-new-file-row input {
  width: 200px;
  padding: 4px 8px;
  font-size: 12px;
  border: 1px solid var(--vp-c-brand-1);
  border-radius: 4px;
  background: var(--vp-c-bg);
  color: var(--vp-c-text-1);
  outline: none;
  font-family: inherit;
}

.editor-container {
  flex: 1;
  overflow: hidden;
}

/* ─── Panel divider ─── */
.panel-divider {
  width: 1px;
  background: var(--vp-c-divider);
  flex-shrink: 0;
}

/* ─── Preview area ─── */
.preview-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  background: var(--vp-c-bg);
}

.preview-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 16px;
  background: var(--vp-c-bg-soft);
  border-bottom: 1px solid var(--vp-c-divider);
  flex-shrink: 0;
}

.preview-header-left {
  display: flex;
  align-items: center;
  gap: 8px;
}

.preview-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--vp-c-text-1);
  letter-spacing: 0.02em;
}

.preview-badge {
  font-size: 10px;
  padding: 2px 8px;
  border-radius: 10px;
  font-weight: 600;
  letter-spacing: 0.02em;
}
.preview-badge.live {
  background: var(--vp-c-tip-soft);
  color: var(--vp-c-tip-1);
}

.preview-stats {
  display: flex;
  align-items: center;
  gap: 6px;
}

.stat-badge {
  font-size: 10px;
  padding: 2px 8px;
  border-radius: 10px;
  font-weight: 600;
  font-family: var(--vp-font-family-mono);
  letter-spacing: 0.01em;
}
.stat-badge.build {
  background: var(--vp-c-brand-soft);
  color: var(--vp-c-brand-1);
}
.stat-badge.render {
  background: var(--vp-c-default-soft);
  color: var(--vp-c-text-2);
}

.error-bar {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 10px 16px;
  font-size: 12px;
  font-family: 'JetBrains Mono', 'Fira Code', monospace;
  color: var(--vp-c-danger-1);
  background: var(--vp-c-danger-soft);
  border-bottom: 1px solid color-mix(in srgb, var(--vp-c-danger-1) 28%, transparent);
  line-height: 1.5;
}
.error-bar svg { flex-shrink: 0; margin-top: 2px; }

.preview-frame {
  flex: 1;
  border: none;
  background: var(--vp-c-bg);
}

.preview-empty {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--vp-c-text-3);
  gap: 8px;
}
.empty-icon { font-size: 32px; }
.preview-empty p { margin: 0; font-size: 14px; }

:deep(.cm-editor) {
  font-family: var(--vp-font-family-mono);
}

:deep(.cm-editor.cm-focused) {
  outline: none;
}

@media (max-width: 1024px) {
  .playground-main {
    flex-direction: column;
  }

  .sidebar {
    width: 100%;
    max-height: 180px;
    border-right: none;
    border-bottom: 1px solid var(--vp-c-divider);
  }

  .panel-divider {
    width: 100%;
    height: 1px;
  }

  .editor-area,
  .preview-area {
    flex: 1 1 50%;
    min-height: 280px;
  }

  .editor-container,
  .preview-frame {
    min-height: 280px;
  }
}

@media (max-width: 768px) {
  .playground-shell {
    overflow: hidden;
  }

  .playground-main {
    position: relative;
  }

  .mobile-files-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    position: absolute;
    top: 10px;
    left: 10px;
    z-index: 32;
    height: 36px;
    min-width: 56px;
    padding: 0 12px;
    border-radius: 18px;
    border: 1px solid var(--vp-c-divider);
    background: var(--vp-c-bg-soft);
    color: var(--vp-c-text-1);
    font-size: 12px;
    font-weight: 600;
  }

  .mobile-overlay {
    display: block;
    position: absolute;
    inset: 0;
    z-index: 30;
    border: none;
    padding: 0;
    background: color-mix(in srgb, var(--vp-c-bg) 55%, transparent);
  }

  .sidebar {
    position: absolute;
    top: 0;
    left: 0;
    bottom: 0;
    width: min(80vw, 280px);
    max-height: none;
    border-right: 1px solid var(--vp-c-divider);
    border-bottom: none;
    transform: translateX(-100%);
    transition: transform 0.2s ease;
    z-index: 31;
    box-shadow: var(--vp-shadow-3);
  }

  .mobile-sidebar-open .sidebar {
    transform: translateX(0);
  }

  .tab-bar {
    padding-left: 72px;
    scroll-snap-type: x mandatory;
  }

  .tab {
    min-height: 36px;
    scroll-snap-align: start;
  }

  .file-item,
  .sidebar-btn,
  .delete-btn {
    min-height: 36px;
  }

  .preview-header {
    position: sticky;
    top: 0;
    z-index: 1;
    padding-right: 10px;
  }

  .preview-stats {
    gap: 4px;
  }

  .stat-badge {
    padding: 2px 6px;
  }
}
</style>

<!-- Unscoped: only active while this component is mounted -->
<style>
html.playground-active .VPSidebar { display: none !important; }
html.playground-active .VPContent.has-sidebar { padding-left: 0 !important; }
html.playground-active .VPFooter { display: none !important; }
html.playground-active .VPDoc .container { max-width: 100% !important; }
html.playground-active .VPDoc .content-container { max-width: 100% !important; }
html.playground-active .VPContent { max-width: 100% !important; padding: 0 !important; }

/* Reset VitePress .vp-doc default styles inside the playground */
html.playground-active .vp-doc {
  padding: 0 !important;
  margin: 0 !important;
}
html.playground-active .vp-doc :where(h1, h2, h3, h4, h5, h6, p, ul, ol, li, div, span, button, input, iframe) {
  margin: unset;
  padding: unset;
  font-size: unset;
  font-weight: unset;
  line-height: unset;
  border: unset;
  color: unset;
}
</style>
