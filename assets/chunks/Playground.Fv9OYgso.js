const __vite__mapDeps=(i,m=__vite__mapDeps,d=(m.f||(m.f=["assets/chunks/index.BFTlCapx.js","assets/chunks/index.D9ZpMuh4.js","assets/chunks/index.BiaBG62Z.js","assets/chunks/index.cMwAWGM0.js","assets/chunks/index.CwHF_7KC.js","assets/chunks/index.-QIpkHM4.js","assets/chunks/index.DwqyHfKm.js","assets/chunks/index.CP2itDtO.js","assets/chunks/index.BlNZ8KkZ.js"])))=>i.map(i=>d[i]);
import{_ as me,az as fe,q as he,v as we,P as S,x as be,c as a,o as i,j as t,e as g,p as c,as as ye,at as ge,Z as Y,F as Z,B as Q,n as z,N as ee,t as h,a0 as _e,E as xe,V as p}from"./framework.IQH0tUiJ.js";const ke=["aria-expanded"],Ee={class:"playground-main"},Ce={id:"playground-sidebar",class:"sidebar"},Le={key:0,class:"new-file-row"},Te={class:"file-list"},Fe=["onClick"],Se={class:"file-name"},Ie=["onClick"],Me={class:"editor-area"},Pe={class:"tab-bar"},je=["onClick"],Ve={class:"tab-name"},De={class:"preview-area"},Oe={class:"preview-header"},Re={key:0,class:"preview-stats"},Ae={class:"stat-badge build"},Be={class:"stat-badge render"},We={key:0,class:"error-bar"},$e=["srcdoc"],Ne={key:2,class:"preview-empty"},Ue={__name:"Playground",setup(Ke){const d=fe({"index.html":`<h1>Hello, {{name}}!</h1>
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
}`,"state.json":JSON.stringify({name:"WebUI",greeting:"This framework rocks!",showGreeting:!0,people:[{name:"Alice",role:"Engineer"},{name:"Bob",role:"Designer"},{name:"Charlie",role:"PM"}]},null,2)}),r=c("index.html"),x=c(""),m=c(""),G=c(!1),k=xe(null),I=c(null),J=c(null);let _=null,M=!1;const E=c(""),C=c(!1),L=c(null),v=c(!1);function H(){return typeof window<"u"&&window.matchMedia("(max-width: 768px)").matches}function te(){v.value=!v.value}function oe(){v.value=!1}function ne(e){r.value=e,H()&&(v.value=!1)}function se(){H()&&(v.value=!0),C.value=!0,S(()=>{L.value&&(L.value.focus(),L.value.select())})}function ae(){const e=E.value.trim();e&&!d[e]&&(d[e]="",r.value=e),E.value="",C.value=!1}function ie(e){e==="index.html"||e==="state.json"||(delete d[e],r.value===e&&(r.value="index.html"))}const P=c(null);let j=null;function re(e){return e.endsWith(".css")?"css":e.endsWith(".json")?"json":"html"}function q(e){return e.endsWith(".css")?"●":e.endsWith(".json")?"◆":"◇"}function X(e){return e.endsWith(".css")?"var(--vp-c-brand-2)":e.endsWith(".json")?"var(--vp-c-warning-1)":"var(--vp-c-brand-1)"}function w(e,o){return getComputedStyle(document.documentElement).getPropertyValue(e).trim()||o}async function V(){if(!P.value)return;const{EditorView:e,keymap:o,lineNumbers:u,highlightActiveLine:s,highlightSpecialChars:b}=await p(async()=>{const{EditorView:n,keymap:l,lineNumbers:K,highlightActiveLine:ue,highlightSpecialChars:ve}=await import("./index.BFTlCapx.js").then(pe=>pe.i);return{EditorView:n,keymap:l,lineNumbers:K,highlightActiveLine:ue,highlightSpecialChars:ve}},__vite__mapDeps([0,1])),{EditorState:R}=await p(async()=>{const{EditorState:n}=await import("./index.D9ZpMuh4.js");return{EditorState:n}},[]),{defaultKeymap:A,history:B,historyKeymap:T}=await p(async()=>{const{defaultKeymap:n,history:l,historyKeymap:K}=await import("./index.BiaBG62Z.js");return{defaultKeymap:n,history:l,historyKeymap:K}},__vite__mapDeps([2,1,0,3])),{oneDark:W}=await p(async()=>{const{oneDark:n}=await import("./index.CwHF_7KC.js");return{oneDark:n}},__vite__mapDeps([4,0,1,3])),{bracketMatching:$}=await p(async()=>{const{bracketMatching:n}=await import("./index.cMwAWGM0.js").then(l=>l.y);return{bracketMatching:n}},__vite__mapDeps([3,1,0])),F=re(r.value);let y;if(F==="css"){const{css:n}=await p(async()=>{const{css:l}=await import("./index.-QIpkHM4.js");return{css:l}},__vite__mapDeps([5,6,3,1,0]));y=n()}else if(F==="json"){const{json:n}=await p(async()=>{const{json:l}=await import("./index.CP2itDtO.js");return{json:l}},__vite__mapDeps([7,6,3,1,0]));y=n()}else{const{html:n}=await p(async()=>{const{html:l}=await import("./index.BlNZ8KkZ.js");return{html:l}},__vite__mapDeps([8,6,3,1,0,5]));y=n()}j&&j.destroy();const N=e.updateListener.of(n=>{n.docChanged&&(d[r.value]=n.state.doc.toString(),le())}),U=document.documentElement.classList.contains("dark"),f=e.theme({"&":{height:"100%",fontSize:"13px",backgroundColor:"var(--vp-c-bg-soft)",color:"var(--vp-c-text-1)"},".cm-scroller":{overflow:"auto"},".cm-gutters":{border:"none",backgroundColor:"var(--vp-c-bg-mute)",color:"var(--vp-c-text-3)"},".cm-content":{fontFamily:"var(--vp-font-family-mono)"},".cm-line":{padding:"0 8px"},".cm-activeLine":{backgroundColor:"var(--vp-c-default-soft)"},".cm-activeLineGutter":{backgroundColor:"var(--vp-c-default-soft)"},".cm-selectionBackground, &.cm-focused .cm-selectionBackground, ::selection":{backgroundColor:"var(--vp-c-brand-soft)"},".cm-cursor, .cm-dropCursor":{borderLeftColor:"var(--vp-c-brand-1)"},".cm-focused":{outline:"none"}});j=new e({state:R.create({doc:d[r.value]||"",extensions:[u(),s(),b(),B(),$(),o.of([...A,...T]),y,...U?[W]:[],N,f]}),parent:P.value})}let D=null;function le(){D&&clearTimeout(D),D=setTimeout(O,150)}async function O(){if(!k.value){m.value="WASM module not loaded yet";return}try{m.value="";const e={};for(const[f,n]of Object.entries(d))f!=="state.json"&&(e[f]=n);const o=d["state.json"]||"{}",u=performance.now(),s=k.value.build_protocol(e,"index.html"),b=performance.now();I.value=(b-u).toFixed(1);const R=performance.now(),A=k.value.render(s,o),B=performance.now();J.value=(B-R).toFixed(1);let T="";for(const[f,n]of Object.entries(d))f.endsWith(".css")&&f!=="state.json"&&(T+=n+`
`);const W=w("--vp-c-bg","#ffffff"),$=w("--vp-c-text-1","#213547"),F=w("--vp-c-divider","#e2e2e3"),y=w("--vp-c-text-1","#213547"),N=w("--vp-font-family-mono","'SFMono-Regular', Consolas, 'Liberation Mono', Menlo, monospace"),U=w("--vp-font-family-base","Inter, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif");x.value=`<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="color-scheme" content="light dark">
  <style>
    *, *::before, *::after { box-sizing: border-box; }
    :root {
      --preview-bg: ${W};
      --preview-text: ${$};
      --preview-border: ${F};
      --preview-heading: ${y};
      --preview-font-base: ${U};
      --preview-font-mono: ${N};
    }
    body {
      font-family: var(--preview-font-base);
      padding: 24px;
      margin: 0;
      color: var(--preview-text);
      background: var(--preview-bg);
      line-height: 1.6;
    }
    h1, h2, h3, h4, h5, h6 {
      color: var(--preview-heading);
      margin-top: 0;
    }
    code, pre {
      font-family: var(--preview-font-mono);
    }
    hr {
      border: 0;
      border-top: 1px solid var(--preview-border);
    }
    ${T}
  </style>
</head>
<body>${A}</body>
</html>`}catch(e){m.value=String(e),x.value=""}}async function ce(){try{const o=await import(new URL("/wasm/webui_wasm.js",window.location.origin).href);await o.default(),k.value=o,G.value=!0,O()}catch(e){m.value="Failed to load WASM module: "+String(e)}}function de(){const e=document.documentElement.classList.contains("dark");e!==M&&(M=e,S(()=>{V(),G.value&&O()}))}return he(r,()=>{S(V)}),we(async()=>{document.documentElement.style.overflow="hidden",document.documentElement.classList.add("playground-active"),M=document.documentElement.classList.contains("dark"),_=new MutationObserver(e=>{for(const o of e)if(o.type==="attributes"&&o.attributeName==="class"){de();break}}),_.observe(document.documentElement,{attributes:!0,attributeFilter:["class"]}),await ce(),await S(),V()}),be(()=>{document.documentElement.style.overflow="",document.documentElement.classList.remove("playground-active"),_&&(_.disconnect(),_=null)}),(e,o)=>(i(),a("div",{class:z(["playground-shell",{"mobile-sidebar-open":v.value}])},[t("button",{class:"mobile-files-btn",type:"button",onClick:te,"aria-expanded":v.value?"true":"false","aria-controls":"playground-sidebar"}," Files ",8,ke),v.value?(i(),a("button",{key:0,class:"mobile-overlay",type:"button","aria-label":"Close file drawer",onClick:oe})):g("",!0),t("div",Ee,[t("div",Ce,[t("div",{class:"sidebar-header"},[o[3]||(o[3]=t("span",null,"EXPLORER",-1)),t("button",{class:"sidebar-btn",onClick:se,title:"New file"},[...o[2]||(o[2]=[t("svg",{viewBox:"0 0 24 24",width:"14",height:"14",fill:"none",stroke:"currentColor","stroke-width":"2"},[t("line",{x1:"12",y1:"5",x2:"12",y2:"19"}),t("line",{x1:"5",y1:"12",x2:"19",y2:"12"})],-1)])])]),C.value?(i(),a("div",Le,[ye(t("input",{ref_key:"newFileInput",ref:L,"onUpdate:modelValue":o[0]||(o[0]=u=>E.value=u),onKeyup:[Y(ae,["enter"]),o[1]||(o[1]=Y(u=>C.value=!1,["escape"]))],placeholder:"filename.html",autofocus:""},null,544),[[ge,E.value]])])):g("",!0),t("div",Te,[(i(!0),a(Z,null,Q(d,(u,s)=>(i(),a("div",{key:s,class:z(["file-item",{active:r.value===s}]),onClick:b=>ne(s)},[t("span",{class:"file-icon",style:ee({color:X(s)})},h(q(s)),5),t("span",Se,h(s),1),s!=="index.html"&&s!=="state.json"?(i(),a("button",{key:0,class:"delete-btn",onClick:_e(b=>ie(s),["stop"]),title:"Delete file"},[...o[4]||(o[4]=[t("svg",{viewBox:"0 0 24 24",width:"12",height:"12",fill:"none",stroke:"currentColor","stroke-width":"2"},[t("line",{x1:"18",y1:"6",x2:"6",y2:"18"}),t("line",{x1:"6",y1:"6",x2:"18",y2:"18"})],-1)])],8,Ie)):g("",!0)],10,Fe))),128))])]),t("div",Me,[t("div",Pe,[(i(!0),a(Z,null,Q(d,(u,s)=>(i(),a("div",{key:s,class:z(["tab",{active:r.value===s}]),onClick:b=>r.value=s},[t("span",{class:"tab-icon",style:ee({color:X(s)})},h(q(s)),5),t("span",Ve,h(s),1)],10,je))),128))]),t("div",{ref_key:"editorContainer",ref:P,class:"editor-container"},null,512)]),o[8]||(o[8]=t("div",{class:"panel-divider"},null,-1)),t("div",De,[t("div",Oe,[o[5]||(o[5]=t("div",{class:"preview-header-left"},[t("span",{class:"preview-title"},"Preview"),t("span",{class:"preview-badge live"},"Live")],-1)),I.value!==null?(i(),a("div",Re,[t("span",Ae,"Build "+h(I.value)+"ms",1),t("span",Be,"Render "+h(J.value)+"ms",1)])):g("",!0)]),m.value?(i(),a("div",We,[o[6]||(o[6]=t("svg",{viewBox:"0 0 24 24",width:"14",height:"14",fill:"none",stroke:"currentColor","stroke-width":"2"},[t("circle",{cx:"12",cy:"12",r:"10"}),t("line",{x1:"15",y1:"9",x2:"9",y2:"15"}),t("line",{x1:"9",y1:"9",x2:"15",y2:"15"})],-1)),t("span",null,h(m.value),1)])):g("",!0),x.value?(i(),a("iframe",{key:1,srcdoc:x.value,class:"preview-frame",sandbox:"allow-scripts"},null,8,$e)):m.value?g("",!0):(i(),a("div",Ne,[...o[7]||(o[7]=[t("div",{class:"empty-icon"},"⚡",-1),t("p",null,"Preview will appear here",-1)])]))])])],2))}},Ge=me(Ue,[["__scopeId","data-v-3c10a18b"]]);export{Ge as default};
