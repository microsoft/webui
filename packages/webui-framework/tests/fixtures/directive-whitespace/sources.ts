// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export interface DirectiveCase {
  name: string;
  source: string;
  kind: 'if' | 'for' | 'keyed';
  conditions: number;
  repeats: number;
  parent: 'ARTICLE' | 'FOOTER';
  message?: boolean;
}

// Escapes preserve the reported bytes independently of Git's checkout line endings.
export const directiveCases: DirectiveCase[] = [
  {
    name: 'if LF opening control',
    source: '<article><if\n condition="enabled"\n><button>{{label}}</button></if></article>',
    kind: 'if', conditions: 1, repeats: 0, parent: 'ARTICLE',
  },
  {
    name: 'if CRLF opening',
    source: '<article><if\r\n condition="enabled"\r\n><button>{{label}}</button></if></article>',
    kind: 'if', conditions: 1, repeats: 0, parent: 'ARTICLE',
  },
  {
    name: 'if LF closing',
    source: '<article><if condition="enabled"><button>{{label}}</button></if\n></article>',
    kind: 'if', conditions: 1, repeats: 0, parent: 'ARTICLE',
  },
  {
    name: 'if space closing',
    source: '<article><if condition="enabled"><button>{{label}}</button></if ></article>',
    kind: 'if', conditions: 1, repeats: 0, parent: 'ARTICLE',
  },
  {
    name: 'nested if LF closing',
    source: '<article><if condition="enabled"><footer><if condition="ready"><button>{{label}}</button></if\n></footer></if></article>',
    kind: 'if', conditions: 2, repeats: 0, parent: 'FOOTER',
  },
  {
    name: 'nested if CRLF closing',
    source: '<article><if condition="enabled"><footer><if condition="ready"><button>{{label}}</button></if\r\n></footer></if></article>',
    kind: 'if', conditions: 2, repeats: 0, parent: 'FOOTER',
  },
  {
    name: 'nested message control',
    source: '<article><if condition="message.enabled"><footer><if condition="message.ready"><button>{{message.label}}</button></if></footer></if></article>',
    kind: 'if', conditions: 2, repeats: 0, parent: 'FOOTER', message: true,
  },
  {
    name: 'nested message LF closing',
    source: '<article><if condition="message.enabled"><footer><if condition="message.ready"><button>{{message.label}}</button></if\n></footer></if></article>',
    kind: 'if', conditions: 2, repeats: 0, parent: 'FOOTER', message: true,
  },
  {
    name: 'nested message space closing',
    source: '<article><if condition="message.enabled"><footer><if condition="message.ready"><button>{{message.label}}</button></if ></footer></if></article>',
    kind: 'if', conditions: 2, repeats: 0, parent: 'FOOTER', message: true,
  },
  {
    name: 'nested message CRLF opening',
    source: '<article><if\r\n condition="message.enabled"\r\n><footer><if\r\n condition="message.ready"\r\n><button>{{message.label}}</button></if></footer></if></article>',
    kind: 'if', conditions: 2, repeats: 0, parent: 'FOOTER', message: true,
  },
  {
    name: 'for LF opening control',
    source: '<article><for\n each="item in items"\n><button data-id="{{item.id}}" @click="{select(item.id)}">{{item.label}}</button></for></article>',
    kind: 'for', conditions: 0, repeats: 1, parent: 'ARTICLE',
  },
  {
    name: 'for CRLF opening',
    source: '<article><for\r\n each="item in items"\r\n><button data-id="{{item.id}}" @click="{select(item.id)}">{{item.label}}</button></for></article>',
    kind: 'for', conditions: 0, repeats: 1, parent: 'ARTICLE',
  },
  {
    name: 'for LF closing',
    source: '<article><for each="item in items"><button data-id="{{item.id}}" @click="{select(item.id)}">{{item.label}}</button></for\n></article>',
    kind: 'for', conditions: 0, repeats: 1, parent: 'ARTICLE',
  },
  {
    name: 'for space closing',
    source: '<article><for each="item in items"><button data-id="{{item.id}}" @click="{select(item.id)}">{{item.label}}</button></for ></article>',
    kind: 'for', conditions: 0, repeats: 1, parent: 'ARTICLE',
  },
  {
    name: 'CRLF between if tags control',
    source: '<article>\r\n<if condition="enabled">\r\n<button>{{label}}</button>\r\n</if>\r\n</article>',
    kind: 'if', conditions: 1, repeats: 0, parent: 'ARTICLE',
  },
  {
    name: 'CRLF between for tags control',
    source: '<article>\r\n<for each="item in items">\r\n<button data-id="{{item.id}}" @click="{select(item.id)}">{{item.label}}</button>\r\n</for>\r\n</article>',
    kind: 'for', conditions: 0, repeats: 1, parent: 'ARTICLE',
  },
];

function keyedSource(space: string, closeSpace: string): string {
  return '<article><p class="literal">&lt;if condition="text"&gt;&lt;for each="text"&gt;</p>'
    + `<if${space}condition="enabled"><section class="rows">`
    + `<for${space}each="item in items">`
    + `<if${space}condition="item.visible"><div key="{{item.id}}" data-id="{{item.id}}">`
    + '<button class="pick" data-id="{{item.id}}" value="{{item.label}}" @click="{select(item.id)}">{{item.label}}</button>'
    + `<if${space}condition="item.detail"><span class="detail">{{item.label}}</span></if${closeSpace}>`
    + `<for${space}each="child in item.children"><button class="child" key="{{child.id}}" data-id="{{child.id}}" @click="{select(child.id)}">{{child.label}}</button></for${closeSpace}>`
    + `</div></if${closeSpace}></for${closeSpace}></section></if${closeSpace}>`
    + `<if${space}condition="ready"><footer><button class="tail" @click="{select('tail')}">{{label}}</button></footer></if${closeSpace}>`
    + '<output>{{selected}}</output></article>';
}

directiveCases.push(
  {
    name: 'nested adjacent keyed control',
    source: keyedSource(' ', ''),
    kind: 'keyed', conditions: 4, repeats: 2, parent: 'ARTICLE',
  },
  {
    name: 'nested adjacent keyed CRLF',
    source: keyedSource('\r\n ', '\r\n'),
    kind: 'keyed', conditions: 4, repeats: 2, parent: 'ARTICLE',
  },
  {
    name: 'nested adjacent keyed tab form-feed',
    source: keyedSource('\t', '\f'),
    kind: 'keyed', conditions: 4, repeats: 2, parent: 'ARTICLE',
  },
);
