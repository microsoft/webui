const __vite__mapDeps=(i,m=__vite__mapDeps,d=(m.f||(m.f=["assets/chunks/index.BFTlCapx.js","assets/chunks/index.D9ZpMuh4.js","assets/chunks/index.DvBK-tPN.js","assets/chunks/index.DjG2toYv.js","assets/chunks/index.CPmxUfdE.js","assets/chunks/index.C50o0Pkr.js","assets/chunks/index.CjGGKgIM.js","assets/chunks/index.Dnly3mrc.js","assets/chunks/index.BJs2f_3A.js"])))=>i.map(i=>d[i]);
import{_ as ie,az as le,q as ce,v as de,P as S,x as ue,c as u,o as v,j as o,e as b,F as ve,B as me,n as pe,p as i,N as fe,t as x,a0 as he,as as ge,at as ye,Z as G,E as we,V as m}from"./framework.IQH0tUiJ.js";const _e={class:"playground-shell"},be={class:"playground-main"},xe={class:"editor-area"},ke={class:"tab-bar"},Ee=["onClick"],Ce={class:"tab-name"},Le=["onClick"],Te={key:0,class:"tab-new-file-row"},Fe={class:"preview-area"},Ie={class:"preview-header"},Se={key:0,class:"preview-stats"},je={class:"stat-badge build"},Pe={class:"stat-badge render"},Ve={key:0,class:"error-bar"},De=["srcdoc"],Me={key:2,class:"preview-empty"},Oe={__name:"Playground",setup(Be){const l=le({"index.html":`<h1>Hello, {{name}}!</h1>
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
}`,"state.json":JSON.stringify({name:"WebUI",greeting:"This framework rocks!",showGreeting:!0,people:[{name:"Alice",role:"Engineer"},{name:"Bob",role:"Designer"},{name:"Charlie",role:"PM"}]},null,2)}),c=i("index.html"),k=i(""),p=i(""),K=i(!1),E=we(null),j=i(null),z=i(null);let f=null,P=!1;const C=i(""),h=i(!1),L=i(null);function J(){h.value=!0,S(()=>{L.value&&(L.value.focus(),L.value.select())})}function q(){const e=C.value.trim();e&&!l[e]&&(l[e]="",c.value=e),C.value="",h.value=!1}function Y(e){e==="index.html"||e==="state.json"||(delete l[e],c.value===e&&(c.value="index.html"))}const V=i(null);let D=null;function Z(e){return e.endsWith(".css")?"css":e.endsWith(".json")?"json":"html"}function Q(e){return e.endsWith(".css")?"●":e.endsWith(".json")?"◆":"◇"}function X(e){return e.endsWith(".css")?"var(--vp-c-brand-2)":e.endsWith(".json")?"var(--vp-c-warning-1)":"var(--vp-c-brand-1)"}function g(e,t){return getComputedStyle(document.documentElement).getPropertyValue(e).trim()||t}async function M(){if(!V.value)return;const{EditorView:e,keymap:t,lineNumbers:d,highlightActiveLine:s,highlightSpecialChars:y}=await m(async()=>{const{EditorView:n,keymap:a,lineNumbers:_,highlightActiveLine:H,highlightSpecialChars:ae}=await import("./index.BFTlCapx.js").then(re=>re.i);return{EditorView:n,keymap:a,lineNumbers:_,highlightActiveLine:H,highlightSpecialChars:ae}},__vite__mapDeps([0,1])),{EditorState:R}=await m(async()=>{const{EditorState:n}=await import("./index.D9ZpMuh4.js");return{EditorState:n}},[]),{defaultKeymap:A,history:W,historyKeymap:T}=await m(async()=>{const{defaultKeymap:n,history:a,historyKeymap:_}=await import("./index.DvBK-tPN.js");return{defaultKeymap:n,history:a,historyKeymap:_}},__vite__mapDeps([2,1,0,3])),{oneDark:$}=await m(async()=>{const{oneDark:n}=await import("./index.CPmxUfdE.js");return{oneDark:n}},__vite__mapDeps([4,0,1,3])),{bracketMatching:F,syntaxHighlighting:N,defaultHighlightStyle:U}=await m(async()=>{const{bracketMatching:n,syntaxHighlighting:a,defaultHighlightStyle:_}=await import("./index.DjG2toYv.js").then(H=>H.y);return{bracketMatching:n,syntaxHighlighting:a,defaultHighlightStyle:_}},__vite__mapDeps([3,1,0])),I=Z(c.value);let r;if(I==="css"){const{css:n}=await m(async()=>{const{css:a}=await import("./index.C50o0Pkr.js");return{css:a}},__vite__mapDeps([5,6,3,1,0]));r=n()}else if(I==="json"){const{json:n}=await m(async()=>{const{json:a}=await import("./index.Dnly3mrc.js");return{json:a}},__vite__mapDeps([7,6,3,1,0]));r=n()}else{const{html:n}=await m(async()=>{const{html:a}=await import("./index.BJs2f_3A.js");return{html:a}},__vite__mapDeps([8,6,3,1,0,5]));r=n()}D&&D.destroy();const w=e.updateListener.of(n=>{n.docChanged&&(l[c.value]=n.state.doc.toString(),ee())}),ne=document.documentElement.classList.contains("dark"),se=e.theme({"&":{height:"100%",fontSize:"13px",backgroundColor:"var(--vp-c-bg-soft)",color:"var(--vp-c-text-1)"},".cm-scroller":{overflow:"auto"},".cm-gutters":{border:"none",backgroundColor:"var(--vp-c-bg-mute)",color:"var(--vp-c-text-3)"},".cm-content":{fontFamily:"var(--vp-font-family-mono)"},".cm-line":{padding:"0 8px"},".cm-activeLine":{backgroundColor:"var(--vp-c-default-soft)"},".cm-activeLineGutter":{backgroundColor:"var(--vp-c-default-soft)"},".cm-selectionBackground, &.cm-focused .cm-selectionBackground, ::selection":{backgroundColor:"var(--vp-c-brand-soft)"},".cm-cursor, .cm-dropCursor":{borderLeftColor:"var(--vp-c-brand-1)"},".cm-focused":{outline:"none"}});D=new e({state:R.create({doc:l[c.value]||"",extensions:[d(),s(),y(),W(),F(),t.of([...A,...T]),r,...ne?[$]:[N(U,{fallback:!0})],w,se]}),parent:V.value})}let O=null;function ee(){O&&clearTimeout(O),O=setTimeout(B,150)}async function B(){if(!E.value){p.value="WASM module not loaded yet";return}try{p.value="";const e={};for(const[r,w]of Object.entries(l))r!=="state.json"&&(e[r]=w);const t=l["state.json"]||"{}",d=performance.now(),s=E.value.build_protocol(e,"index.html"),y=performance.now();j.value=(y-d).toFixed(1);const R=performance.now(),A=E.value.render(s,t),W=performance.now();z.value=(W-R).toFixed(1);let T="";for(const[r,w]of Object.entries(l))r.endsWith(".css")&&r!=="state.json"&&(T+=w+`
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
</html>`}catch(e){p.value=String(e),k.value=""}}async function te(){try{const t=await import(new URL("/wasm/webui_wasm.js",window.location.origin).href);await t.default(),E.value=t,K.value=!0,B()}catch(e){p.value="Failed to load WASM module: "+String(e)}}function oe(){const e=document.documentElement.classList.contains("dark");e!==P&&(P=e,S(()=>{M(),K.value&&B()}))}return ce(c,()=>{S(M)}),de(async()=>{document.documentElement.style.overflow="hidden",document.documentElement.classList.add("playground-active"),P=document.documentElement.classList.contains("dark"),f=new MutationObserver(e=>{for(const t of e)if(t.type==="attributes"&&t.attributeName==="class"){oe();break}}),f.observe(document.documentElement,{attributes:!0,attributeFilter:["class"]}),await te(),await S(),M()}),ue(()=>{document.documentElement.style.overflow="",document.documentElement.classList.remove("playground-active"),f&&(f.disconnect(),f=null)}),(e,t)=>(v(),u("div",_e,[o("div",be,[o("div",xe,[o("div",ke,[(v(!0),u(ve,null,me(l,(d,s)=>(v(),u("div",{key:s,class:pe(["tab",{active:c.value===s}]),onClick:y=>c.value=s},[o("span",{class:"tab-icon",style:fe({color:X(s)})},x(Q(s)),5),o("span",Ce,x(s),1),s!=="index.html"&&s!=="state.json"?(v(),u("button",{key:0,class:"tab-close-btn",onClick:he(y=>Y(s),["stop"]),title:"Close file"},[...t[3]||(t[3]=[o("svg",{viewBox:"0 0 24 24",width:"10",height:"10",fill:"none",stroke:"currentColor","stroke-width":"2"},[o("line",{x1:"18",y1:"6",x2:"6",y2:"18"}),o("line",{x1:"6",y1:"6",x2:"18",y2:"18"})],-1)])],8,Le)):b("",!0)],10,Ee))),128)),o("button",{class:"tab-add-btn",onClick:J,title:"New file"},[...t[4]||(t[4]=[o("svg",{viewBox:"0 0 24 24",width:"14",height:"14",fill:"none",stroke:"currentColor","stroke-width":"2"},[o("line",{x1:"12",y1:"5",x2:"12",y2:"19"}),o("line",{x1:"5",y1:"12",x2:"19",y2:"12"})],-1)])])]),h.value?(v(),u("div",Te,[ge(o("input",{ref_key:"newFileInput",ref:L,"onUpdate:modelValue":t[0]||(t[0]=d=>C.value=d),onKeyup:[G(q,["enter"]),t[1]||(t[1]=G(d=>h.value=!1,["escape"]))],onBlur:t[2]||(t[2]=d=>h.value=!1),placeholder:"filename.html",autofocus:""},null,544),[[ye,C.value]])])):b("",!0),o("div",{ref_key:"editorContainer",ref:V,class:"editor-container"},null,512)]),t[8]||(t[8]=o("div",{class:"panel-divider"},null,-1)),o("div",Fe,[o("div",Ie,[t[5]||(t[5]=o("div",{class:"preview-header-left"},[o("span",{class:"preview-title"},"Preview"),o("span",{class:"preview-badge live"},"Live")],-1)),j.value!==null?(v(),u("div",Se,[o("span",je,"Build "+x(j.value)+"ms",1),o("span",Pe,"Render "+x(z.value)+"ms",1)])):b("",!0)]),p.value?(v(),u("div",Ve,[t[6]||(t[6]=o("svg",{viewBox:"0 0 24 24",width:"14",height:"14",fill:"none",stroke:"currentColor","stroke-width":"2"},[o("circle",{cx:"12",cy:"12",r:"10"}),o("line",{x1:"15",y1:"9",x2:"9",y2:"15"}),o("line",{x1:"9",y1:"9",x2:"15",y2:"15"})],-1)),o("span",null,x(p.value),1)])):b("",!0),k.value?(v(),u("iframe",{key:1,srcdoc:k.value,class:"preview-frame",sandbox:"allow-scripts"},null,8,De)):p.value?b("",!0):(v(),u("div",Me,[...t[7]||(t[7]=[o("div",{class:"empty-icon"},"⚡",-1),o("p",null,"Preview will appear here",-1)])]))])])]))}},Ae=ie(Oe,[["__scopeId","data-v-70f90b77"]]);export{Ae as default};
