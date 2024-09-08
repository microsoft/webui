import assert from 'node:assert'
import { describe, it } from 'node:test'
import { ParseError, createBuildTimeProtocol } from './replay_protocol.js'

describe('createBuildTimeProtocol', () => {
  it('should process raw text correctly', () => {
    const htmlContent = '<div>Hello World</div>'
    const result = createBuildTimeProtocol(htmlContent)
    assert.deepStrictEqual(result.streams, [
      { type: 'raw', value: '<div>Hello World</div>' },
    ])
  })

  it('should fail with invalid markup', () => {
    const htmlContent = '<div><span>Hello World'
    assert.throws(
      () => createBuildTimeProtocol(htmlContent),
      ParseError,
      'Invalid markup',
    )
  })

  it('should process complex raw text correctly', () => {
    const htmlContent =
      '<!DOCTYPE html><html><head><meta chartset="utf-8"><style>body {margin: 0;}</style></head><body><div>Hello World<span></span></div><script type="module" src="./hello.js"></script></body></html>'
    const result = createBuildTimeProtocol(htmlContent)
    assert.deepStrictEqual(result.streams, [
      {
        type: 'raw',
        value:
          '<!DOCTYPE html><html><head><meta chartset="utf-8" ><style>body {margin: 0;}</style></head><body><div>Hello World<span></span></div><script type="module"  src="./hello.js" ></script></body></html>',
      },
    ])
  })

  it('should process signal attribute correctly', () => {
    const htmlContent = '<div f-signal="testSignal"></div>'
    const result = createBuildTimeProtocol(htmlContent)
    assert.deepStrictEqual(result.streams, [
      { type: 'raw', value: '<div f-signal="testSignal" >' },
      { type: 'signal', value: 'testSignal' },
      { type: 'raw', value: '</div>' },
    ])
  })

  it('should process repeat attribute correctly', () => {
    const htmlContent = '<div f-repeat="testRepeat"><span>Item</span></div>'
    const result = createBuildTimeProtocol(htmlContent)
    assert.deepStrictEqual(result.streams, [
      { type: 'raw', value: '<div f-repeat="testRepeat" >' },
      { type: 'repeat', value: 'testRepeat', template: 'repeat-1' },
      { type: 'raw', value: '</div>' },
    ])
    assert.deepStrictEqual(result.templates['repeat-1'], [
      { type: 'raw', value: '<span>Item</span>' },
    ])
  })

  it('should process nothing with empty repeat attribute', () => {
    const htmlContent = '<div f-repeat="testRepeat"></div>'
    const result = createBuildTimeProtocol(htmlContent)
    assert.deepStrictEqual(result.streams, [
      { type: 'raw', value: '<div f-repeat="testRepeat" ></div>' },
    ])
  })

  it('should process nested repeat attributes correctly', () => {
    const htmlContent = `
      <div f-repeat="outerRepeat">
        <div f-repeat="innerRepeat">
          <span>Inner Item</span>
        </div>
      </div>
    `
    const result = createBuildTimeProtocol(htmlContent)

    assert.deepStrictEqual(result.streams, [
      { type: 'raw', value: '<div f-repeat="outerRepeat" >' },
      { type: 'repeat', value: 'outerRepeat', template: 'repeat-1' },
      { type: 'raw', value: '</div>' },
    ])

    assert.deepStrictEqual(result.templates['repeat-1'], [
      { type: 'raw', value: '<div f-repeat="innerRepeat" >' },
      { type: 'repeat', value: 'innerRepeat', template: 'repeat-2' },
      { type: 'raw', value: '</div>' },
    ])

    assert.deepStrictEqual(result.templates['repeat-2'], [
      { type: 'raw', value: '<span>Inner Item</span>' },
    ])
  })

  it('should process when attribute correctly', () => {
    const htmlContent = '<div f-when="testWhen"></div>'
    const result = createBuildTimeProtocol(htmlContent)
    assert.deepStrictEqual(result.streams, [
      { type: 'raw', value: '<div f-when="testWhen" ' },
      { type: 'when', value: 'testWhen' },
      { type: 'raw', value: '></div>' },
    ])
  })

  it('should process custom attributes correctly', () => {
    const htmlContent = '<div f-custom="customValue"></div>'
    const result = createBuildTimeProtocol(htmlContent)
    assert.deepStrictEqual(result.streams, [
      { type: 'raw', value: '<div f-custom="customValue" ' },
      { type: 'attribute', key: 'custom', value: 'customValue' },
      { type: 'raw', value: '></div>' },
    ])
  })

  it('should process missing web components correctly', () => {
    const htmlContent = '<custom-element></custom-element>'
    const result = createBuildTimeProtocol(htmlContent)
    assert.deepStrictEqual(result, {
      streams: [
        { type: 'raw', value: '<custom-element></custom-element>' },
      ],
      templates: {},
    })
  })

  it('should process available web components', () => {
    const htmlContent = '<custom-element></custom-element>'
    const result = createBuildTimeProtocol(htmlContent, {
      'custom-element': {
        template: '<div>Custom Element</div>',
      },
    })
    assert.deepStrictEqual(result, {
      streams: [
        { type: 'raw', value: '<custom-element><template shadowrootmode="open">' },
        { type: 'component', value: 'custom-element', css: undefined },
        { type: 'raw', value: '</template></custom-element>' },
      ],
      templates: {
        'custom-element': [
          { type: 'raw', value: '<div>Custom Element</div>' },
        ],
      },
    })
  })

  it('should process available web components with slots', () => {
    const htmlContent = '<custom-element appearance="subtle">Hello World</custom-element>'
    const result = createBuildTimeProtocol(htmlContent, {
      'custom-element': {
        template: '<slot></slot>',
      },
    })
    assert.deepStrictEqual(result, {
      streams: [
        { type: 'raw', value: '<custom-element appearance="subtle" ><template shadowrootmode="open">' },
        { type: 'component', value: 'custom-element', css: undefined },
        { type: 'raw', value: '</template>Hello World</custom-element>' },
      ],
      templates: {
        'custom-element': [
          { type: 'raw', value: '<slot></slot>' },
        ],
      },
    })
  })

  it('should process available web components with multiple slots and attributes', () => {
    const htmlContent =
      '<custom-element><span slot="first" f-signal="firstname">Hello</span><span f-signal="lastname">World</span></custom-element>'
    const result = createBuildTimeProtocol(htmlContent, {
      'custom-element': {
        template: '<slot name="first"></slot><slot></slot>',
      },
    })
    assert.deepStrictEqual(result, {
      streams: [
        { type: 'raw', value: '<custom-element><template shadowrootmode="open">' },
        { type: 'component', value: 'custom-element', css: undefined },
        { type: 'raw', value: '</template><span slot="first"  f-signal="firstname" >' },
        { type: 'signal', value: 'firstname' },
        { type: 'raw', value: '</span><span f-signal="lastname" >' },
        { type: 'signal', value: 'lastname' },
        { type: 'raw', value: '</span></custom-element>' },
      ],
      templates: {
        'custom-element': [
          { type: 'raw', value: '<slot name="first" ></slot><slot></slot>' },
        ],
      },
    })
  })

  it('handle multiple nested web components', () => {
    const htmlContent = '<div f-repeat="data"><custom-element><custom-button>Ok</custom-button></custom-element></div>'
    const result = createBuildTimeProtocol(htmlContent, {
      'custom-element': {
        template: '<custom-child></custom-child><slot></slot>',
      },
      'custom-button': {
        template: '<slot></slot>',
      },
      'custom-child': {
        template: '<h1>Hello World!</h1>',
      },
    })
    assert.deepStrictEqual(result, {
      streams: [
        { type: 'raw', value: '<div f-repeat="data" >' },
        { type: 'repeat', value: 'data', template: 'repeat-1' },
        { type: 'raw', value: '</div>' },
      ],
      templates: {
        'custom-button': [
          { type: 'raw', value: '<slot></slot>' },
        ],
        'custom-element': [
          { type: 'raw', value: '<custom-child><template shadowrootmode="open">' },
          { type: 'component', value: 'custom-child', css: undefined },
          { type: 'raw', value: '</template></custom-child><slot></slot>' },
        ],
        'custom-child': [
          { type: 'raw', value: '<h1>Hello World!</h1>' },
        ],
        'repeat-1': [
          { type: 'raw', value: '<custom-element><template shadowrootmode="open">' },
          { type: 'component', value: 'custom-element', css: undefined },
          { type: 'raw', value: '</template><custom-button><template shadowrootmode="open">' },
          { type: 'component', value: 'custom-button', css: undefined },
          { type: 'raw', value: '</template>Ok</custom-button></custom-element>' },
        ],
      },
    })
  })

  it('should process handlebars from text as signals', () => {
    const htmlContent = '<div>{{first}} and {{last}} ({{{html}}})</div>'
    const result = createBuildTimeProtocol(htmlContent)
    assert.deepStrictEqual(result.streams, [
      { type: 'raw', value: '<div>' },
      { type: 'signal', value: 'first' },
      { type: 'raw', value: ' and ' },
      { type: 'signal', value: 'last' },
      { type: 'raw', value: ' (' },
      { type: 'signal', value: 'html', raw: true },
      { type: 'raw', value: ')</div>' },
    ])
  })
})
