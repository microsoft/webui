const __vite__mapDeps=(i,m=__vite__mapDeps,d=(m.f||(m.f=["assets/chunks/index.BFTlCapx.js","assets/chunks/index.D9ZpMuh4.js","assets/chunks/index.DvBK-tPN.js","assets/chunks/index.DjG2toYv.js","assets/chunks/index.CPmxUfdE.js","assets/chunks/index.C50o0Pkr.js","assets/chunks/index.CjGGKgIM.js","assets/chunks/index.Dnly3mrc.js","assets/chunks/index.BJs2f_3A.js"])))=>i.map(i=>d[i]);
import{_ as ie,az as le,q as ce,v as de,P as S,x as ue,c as u,o as v,j as t,e as _,F as ve,B as me,n as fe,p as l,N as pe,t as x,a0 as he,as as ge,at as ye,Z as G,E as we,V as m}from"./framework.B7Jkh64R.js";const be={class:"playground-shell"},_e={class:"playground-main"},xe={class:"editor-area"},ke={class:"tab-bar"},Ee=["onClick"],Ce={class:"tab-name"},Le=["onClick"],Te={key:0,class:"tab-new-file-row"},Fe={class:"preview-area"},Ie={class:"preview-header"},Se={key:0,class:"preview-stats"},je={class:"stat-badge build"},Pe={class:"stat-badge render"},Ve={key:0,class:"error-bar"},De=["srcdoc"],Me={key:2,class:"preview-empty"},Oe={__name:"Playground",setup(Be){const c=le({"index.html":`<h1>Hello, {{name}}!</h1>
<p>Welcome to the WebUI Playground.</p>

<if condition="showGreeting">
  <p>{{greeting}}</p>
</if>

<h2>Team</h2>
<for each="person in people">
  <person-card>{{person.name}} - {{person.role}}</person-card>
</for>`,"person-card.html":`<div class="card">
  <slot></slot>
</div>`,"person-card.css":`.card {
  padding: 8px 16px;
  margin: 4px 0;
  border-left: 3px solid #646cff;
}`,"state.json":JSON.stringify({name:"WebUI",greeting:"This framework rocks!",showGreeting:!0,people:[{name:"Alice",role:"Engineer"},{name:"Bob",role:"Designer"},{name:"Charlie",role:"PM"}]},null,2)}),d=l("index.html"),k=l(""),f=l(""),K=l(!1),E=we(null),j=l(null),z=l(null);let p=null,P=!1;const C=l(""),h=l(!1),L=l(null);function J(){h.value=!0,S(()=>{L.value&&(L.value.focus(),L.value.select())})}function q(){const e=C.value.trim();e&&!c[e]&&(c[e]="",d.value=e),C.value="",h.value=!1}function Y(e){e==="index.html"||e==="state.json"||(delete c[e],d.value===e&&(d.value="index.html"))}const V=l(null);let D=null;function Z(e){return e.endsWith(".css")?"css":e.endsWith(".json")?"json":"html"}function Q(e){return e.endsWith(".css")?"●":e.endsWith(".json")?"◆":"◇"}function X(e){return e.endsWith(".css")?"var(--vp-c-brand-2)":e.endsWith(".json")?"var(--vp-c-warning-1)":"var(--vp-c-brand-1)"}function g(e,o){return getComputedStyle(document.documentElement).getPropertyValue(e).trim()||o}async function M(){if(!V.value)return;const{EditorView:e,keymap:o,lineNumbers:a,highlightActiveLine:s,highlightSpecialChars:y}=await m(async()=>{const{EditorView:n,keymap:r,lineNumbers:b,highlightActiveLine:H,highlightSpecialChars:ae}=await import("./index.BFTlCapx.js").then(re=>re.i);return{EditorView:n,keymap:r,lineNumbers:b,highlightActiveLine:H,highlightSpecialChars:ae}},__vite__mapDeps([0,1])),{EditorState:R}=await m(async()=>{const{EditorState:n}=await import("./index.D9ZpMuh4.js");return{EditorState:n}},[]),{defaultKeymap:A,history:W,historyKeymap:T}=await m(async()=>{const{defaultKeymap:n,history:r,historyKeymap:b}=await import("./index.DvBK-tPN.js");return{defaultKeymap:n,history:r,historyKeymap:b}},__vite__mapDeps([2,1,0,3])),{oneDark:$}=await m(async()=>{const{oneDark:n}=await import("./index.CPmxUfdE.js");return{oneDark:n}},__vite__mapDeps([4,0,1,3])),{bracketMatching:F,syntaxHighlighting:N,defaultHighlightStyle:U}=await m(async()=>{const{bracketMatching:n,syntaxHighlighting:r,defaultHighlightStyle:b}=await import("./index.DjG2toYv.js").then(H=>H.y);return{bracketMatching:n,syntaxHighlighting:r,defaultHighlightStyle:b}},__vite__mapDeps([3,1,0])),I=Z(d.value);let i;if(I==="css"){const{css:n}=await m(async()=>{const{css:r}=await import("./index.C50o0Pkr.js");return{css:r}},__vite__mapDeps([5,6,3,1,0]));i=n()}else if(I==="json"){const{json:n}=await m(async()=>{const{json:r}=await import("./index.Dnly3mrc.js");return{json:r}},__vite__mapDeps([7,6,3,1,0]));i=n()}else{const{html:n}=await m(async()=>{const{html:r}=await import("./index.BJs2f_3A.js");return{html:r}},__vite__mapDeps([8,6,3,1,0,5]));i=n()}D&&D.destroy();const w=e.updateListener.of(n=>{n.docChanged&&(c[d.value]=n.state.doc.toString(),ee())}),ne=document.documentElement.classList.contains("dark"),se=e.theme({"&":{height:"100%",fontSize:"13px",backgroundColor:"var(--vp-c-bg-soft)",color:"var(--vp-c-text-1)"},".cm-scroller":{overflow:"auto"},".cm-gutters":{border:"none",backgroundColor:"var(--vp-c-bg-mute)",color:"var(--vp-c-text-3)"},".cm-content":{fontFamily:"var(--vp-font-family-mono)"},".cm-line":{padding:"0 8px"},".cm-activeLine":{backgroundColor:"var(--vp-c-default-soft)"},".cm-activeLineGutter":{backgroundColor:"var(--vp-c-default-soft)"},".cm-selectionBackground, &.cm-focused .cm-selectionBackground, ::selection":{backgroundColor:"var(--vp-c-brand-soft)"},".cm-cursor, .cm-dropCursor":{borderLeftColor:"var(--vp-c-brand-1)"},".cm-focused":{outline:"none"}});D=new e({state:R.create({doc:c[d.value]||"",extensions:[a(),s(),y(),W(),F(),o.of([...A,...T]),i,...ne?[$]:[N(U,{fallback:!0})],w,se]}),parent:V.value})}let O=null;function ee(){O&&clearTimeout(O),O=setTimeout(B,150)}async function B(){if(!E.value){f.value="WASM module not loaded yet";return}try{f.value="";const e={};for(const[i,w]of Object.entries(c))i!=="state.json"&&(e[i]=w);const o=c["state.json"]||"{}",a=performance.now(),s=E.value.build_protocol(e,"index.html"),y=performance.now();j.value=(y-a).toFixed(1);const R=performance.now(),A=E.value.render(s,o,"index.html","/"),W=performance.now();z.value=(W-R).toFixed(1);let T="";for(const[i,w]of Object.entries(c))i.endsWith(".css")&&i!=="state.json"&&(T+=w+`
`);const $=g("--vp-c-bg","#ffffff"),F=g("--vp-c-text-1","#213547"),N=g("--vp-c-divider","#e2e2e3"),U=g("--vp-font-family-mono","'SFMono-Regular', Consolas, 'Liberation Mono', Menlo, monospace"),I=g("--vp-font-family-base","Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif");k.value=`<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="color-scheme" content="light dark">
  <style>
    *, *::before, *::after { box-sizing: border-box; }
    body {
      font-family: ${I};
      padding: 24px;
      margin: 0;
      color: ${F};
      background: ${$};
      line-height: 1.6;
    }
    h1, h2, h3, h4, h5, h6 {
      color: ${F};
      margin-top: 0;
    }
    code, pre {
      font-family: ${U};
    }
    hr {
      border: 0;
      border-top: 1px solid ${N};
    }
    ${T}
  </style>
</head>
<body>${A}</body>
</html>`}catch(e){f.value=String(e),k.value=""}}async function te(){try{const e="/webui/",a=await import(new URL(`${e}wasm/webui_wasm.js`,window.location.origin).href);await a.default(),E.value=a,K.value=!0,B()}catch(e){f.value="Failed to load WASM module: "+String(e)}}function oe(){const e=document.documentElement.classList.contains("dark");e!==P&&(P=e,S(()=>{M(),K.value&&B()}))}return ce(d,()=>{S(M)}),de(async()=>{document.documentElement.style.overflow="hidden",document.documentElement.classList.add("playground-active"),P=document.documentElement.classList.contains("dark"),p=new MutationObserver(e=>{for(const o of e)if(o.type==="attributes"&&o.attributeName==="class"){oe();break}}),p.observe(document.documentElement,{attributes:!0,attributeFilter:["class"]}),await te(),await S(),M()}),ue(()=>{document.documentElement.style.overflow="",document.documentElement.classList.remove("playground-active"),p&&(p.disconnect(),p=null)}),(e,o)=>(v(),u("div",be,[t("div",_e,[t("div",xe,[t("div",ke,[(v(!0),u(ve,null,me(c,(a,s)=>(v(),u("div",{key:s,class:fe(["tab",{active:d.value===s}]),onClick:y=>d.value=s},[t("span",{class:"tab-icon",style:pe({color:X(s)})},x(Q(s)),5),t("span",Ce,x(s),1),s!=="index.html"&&s!=="state.json"?(v(),u("button",{key:0,class:"tab-close-btn",onClick:he(y=>Y(s),["stop"]),title:"Close file"},[...o[3]||(o[3]=[t("svg",{viewBox:"0 0 24 24",width:"10",height:"10",fill:"none",stroke:"currentColor","stroke-width":"2"},[t("line",{x1:"18",y1:"6",x2:"6",y2:"18"}),t("line",{x1:"6",y1:"6",x2:"18",y2:"18"})],-1)])],8,Le)):_("",!0)],10,Ee))),128)),t("button",{class:"tab-add-btn",onClick:J,title:"New file"},[...o[4]||(o[4]=[t("svg",{viewBox:"0 0 24 24",width:"14",height:"14",fill:"none",stroke:"currentColor","stroke-width":"2"},[t("line",{x1:"12",y1:"5",x2:"12",y2:"19"}),t("line",{x1:"5",y1:"12",x2:"19",y2:"12"})],-1)])])]),h.value?(v(),u("div",Te,[ge(t("input",{ref_key:"newFileInput",ref:L,"onUpdate:modelValue":o[0]||(o[0]=a=>C.value=a),onKeyup:[G(q,["enter"]),o[1]||(o[1]=G(a=>h.value=!1,["escape"]))],onBlur:o[2]||(o[2]=a=>h.value=!1),placeholder:"filename.html",autofocus:""},null,544),[[ye,C.value]])])):_("",!0),t("div",{ref_key:"editorContainer",ref:V,class:"editor-container"},null,512)]),o[8]||(o[8]=t("div",{class:"panel-divider"},null,-1)),t("div",Fe,[t("div",Ie,[o[5]||(o[5]=t("div",{class:"preview-header-left"},[t("span",{class:"preview-title"},"Preview"),t("span",{class:"preview-badge live"},"Live")],-1)),j.value!==null?(v(),u("div",Se,[t("span",je,"Build "+x(j.value)+"ms",1),t("span",Pe,"Render "+x(z.value)+"ms",1)])):_("",!0)]),f.value?(v(),u("div",Ve,[o[6]||(o[6]=t("svg",{viewBox:"0 0 24 24",width:"14",height:"14",fill:"none",stroke:"currentColor","stroke-width":"2"},[t("circle",{cx:"12",cy:"12",r:"10"}),t("line",{x1:"15",y1:"9",x2:"9",y2:"15"}),t("line",{x1:"9",y1:"9",x2:"15",y2:"15"})],-1)),t("span",null,x(f.value),1)])):_("",!0),k.value?(v(),u("iframe",{key:1,srcdoc:k.value,class:"preview-frame",sandbox:"allow-scripts"},null,8,De)):f.value?_("",!0):(v(),u("div",Me,[...o[7]||(o[7]=[t("div",{class:"empty-icon"},"⚡",-1),t("p",null,"Preview will appear here",-1)])]))])])]))}},Ae=ie(Oe,[["__scopeId","data-v-be0fcf6f"]]);export{Ae as default};
