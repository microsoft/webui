import type { BuildTimeRenderingProtocol, BuildTimeRenderingStream } from '@btjs/protocol-js'
import assert from 'node:assert'
import { describe, it } from 'node:test'
import { handleBTR } from './index.js'

function createMockServerHandler() {
  const serverHandler = {
    html: '',
    write: (value: string) => {
      serverHandler.html += value
    },
    end: () => {},
  }
  return serverHandler
}

describe('handleBTR', () => {
  it('should process raw text correctly', () => {
    const protocol: BuildTimeRenderingProtocol = {
      streams: [
        { type: 'raw', value: '<div>Hello World</div>' },
      ],
      templates: {},
    }
    const state = {}
    const serverHandler = createMockServerHandler()
    handleBTR(protocol, state, serverHandler)
    assert.equal(serverHandler.html, '<!DOCTYPE html><html><div>Hello World</div></html>')
  })

  it('should process signal attribute correctly', () => {
    const protocol: BuildTimeRenderingProtocol = {
      streams: [
        { type: 'raw', value: '<div f-signal="testSignal" >' },
        { type: 'signal', value: 'testSignal' },
        { type: 'raw', value: '</div>' },
      ],
      templates: {},
    }
    const state = { testSignal: 'Hello World' }
    const serverHandler = createMockServerHandler()
    handleBTR(protocol, state, serverHandler)
    assert.equal(serverHandler.html, '<!DOCTYPE html><html><div f-signal="testSignal" >Hello World</div></html>')
  })

  it('should process repeat attribute correctly', () => {
    const protocol: BuildTimeRenderingProtocol = {
      streams: [
        { type: 'raw', value: '<div f-repeat="testRepeat" >' },
        { type: 'repeat', value: 'testRepeat', template: 'template1' },
        { type: 'raw', value: '</div>' },
      ],
      templates: {
        template1: [
          { type: 'raw', value: '<span>Item</span>' },
        ],
      },
    }
    const state = { testRepeat: ['Item1', 'Item2'] }
    const serverHandler = createMockServerHandler()
    handleBTR(protocol, state, serverHandler)
    assert.equal(
      serverHandler.html,
      '<!DOCTYPE html><html><div f-repeat="testRepeat" ><span>Item</span><span>Item</span></div></html>',
    )
  })

  it('should process nested repeat attributes correctly', () => {
    const protocol: BuildTimeRenderingProtocol = {
      streams: [
        { type: 'raw', value: '<div f-repeat="outerRepeat" >' },
        { type: 'repeat', value: 'outerRepeat', template: 'outerTemplate' },
        { type: 'raw', value: '</div>' },
      ],
      templates: {
        outerTemplate: [
          { type: 'raw', value: '<div f-repeat="innerRepeat" >' },
          { type: 'repeat', value: 'innerRepeat', template: 'innerTemplate' },
          { type: 'raw', value: '</div>' },
        ],
        innerTemplate: [
          { type: 'raw', value: '<span>Inner Item</span>' },
        ],
      },
    }
    const state = { outerRepeat: [{ innerRepeat: ['Item1', 'Item2'] }, { innerRepeat: ['Item3', 'Item4'] }] }
    const serverHandler = createMockServerHandler()
    handleBTR(protocol, state, serverHandler)
    assert.equal(
      serverHandler.html,
      '<!DOCTYPE html><html><div f-repeat="outerRepeat" ><div f-repeat="innerRepeat" ><span>Inner Item</span><span>Inner Item</span></div><div f-repeat="innerRepeat" ><span>Inner Item</span><span>Inner Item</span></div></div></html>',
    )
  })

  it('should process web component correctly', () => {
    const protocol: BuildTimeRenderingProtocol = {
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
    }
    const state = {}
    const serverHandler = createMockServerHandler()
    handleBTR(protocol, state, serverHandler)
    assert.equal(
      serverHandler.html,
      '<!DOCTYPE html><html><custom-element><template shadowrootmode="open"><div>Custom Element</div></template></custom-element></html>',
    )
  })

  it('should process available web components with slots', () => {
    const protocol: BuildTimeRenderingProtocol = {
      streams: [
        { type: 'raw', value: '<custom-element appearance="subtle"><template shadowrootmode="open">' },
        { type: 'component', value: 'custom-element', css: undefined },
        { type: 'raw', value: '</template>Hello World</custom-element>' },
      ],
      templates: {
        'custom-element': [
          { type: 'raw', value: '<slot></slot>' },
        ],
      },
    }
    const serverHandler = createMockServerHandler()
    const state = {}
    handleBTR(protocol, state, serverHandler)
    assert.equal(
      serverHandler.html,
      '<!DOCTYPE html><html><custom-element appearance="subtle"><template shadowrootmode="open"><slot></slot></template>Hello World</custom-element></html>',
    )
  })

  it('handle multiple nested web components', () => {
    const protocol: BuildTimeRenderingProtocol = {
      streams: [
        { type: 'raw', value: '<div f-repeat="data" >' },
        { type: 'repeat', value: 'data', template: 'templateRepeat' },
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
        'templateRepeat': [
          { type: 'raw', value: '<custom-element><template shadowrootmode="open">' },
          { type: 'component', value: 'custom-element', css: 'custom-element.css' },
          { type: 'raw', value: '</template><custom-button><template shadowrootmode="open">' },
          { type: 'component', value: 'custom-button', css: undefined },
          { type: 'raw', value: '</template>Ok</custom-button></custom-element>' },
        ],
      },
    }
    const serverHandler = createMockServerHandler()
    const state = {
      data: ['Item1'],
    }
    handleBTR(protocol, state, serverHandler)
    assert.equal(
      serverHandler.html,
      '<!DOCTYPE html>' +
        '<html>' +
        '<div f-repeat="data" >' +
        '<custom-element>' +
        '<template shadowrootmode="open">' +
        '<link rel="stylesheet" href="./custom-element.css">' +
        '<custom-child>' +
        '<template shadowrootmode="open">' +
        '<h1>Hello World!</h1>' +
        '</template>' +
        '</custom-child>' +
        '<slot></slot>' +
        '</template>' +
        '<custom-button>' +
        '<template shadowrootmode="open">' +
        '<slot></slot>' +
        '</template>' +
        'Ok' +
        '</custom-button>' +
        '</custom-element>' +
        '</div>' +
        '</html>',
    )
  })

  it('should process raw signals and not escape them', () => {
    const protocol: BuildTimeRenderingProtocol = {
      streams: [
        { type: 'signal', value: 'html' },
        { type: 'signal', value: 'html', raw: true },
      ],
      templates: {},
    }
    const state = { html: '<strong>hi</strong>' }
    const serverHandler = createMockServerHandler()
    handleBTR(protocol, state, serverHandler)
    assert.equal(
      serverHandler.html,
      '<!DOCTYPE html><html>&lt;strong&gt;hi&lt;/strong&gt;<strong>hi</strong></html>',
    )
  })
})
