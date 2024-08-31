import assert from 'node:assert'
import { describe, it } from 'node:test'
import { createBuildTimeProtocol } from './replay_protocol.js'

describe('createBuildTimeProtocol', () => {
  it('should process raw text correctly', () => {
    const htmlContent = '<div>Hello World</div>'
    const result = createBuildTimeProtocol(htmlContent)
    assert.deepStrictEqual(result.streams, [
      { type: 'raw', value: '<div>Hello World</div>' },
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
    const templateId = Object.keys(result.templates)[0]
    assert.deepStrictEqual(result.streams, [
      { type: 'raw', value: '<div f-repeat="testRepeat" >' },
      { type: 'repeat', value: 'testRepeat', template: templateId },
      { type: 'raw', value: '</div>' },
    ])
    assert.deepStrictEqual(result.templates[templateId], [
      { type: 'raw', value: '<span>Item</span>' },
    ])
  })

  it('should process nothing with empty repeat attribute', () => {
    const htmlContent = '<div f-repeat="testRepeat"></div>'
    const result = createBuildTimeProtocol(htmlContent)
    const templateId = Object.keys(result.templates)[0]
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
    const outerTemplateId = Object.keys(result.templates)[0]
    const innerTemplateId = Object.keys(result.templates)[1]

    assert.deepStrictEqual(result.streams, [
      { type: 'raw', value: '<div f-repeat="outerRepeat" >' },
      { type: 'repeat', value: 'outerRepeat', template: outerTemplateId },
      { type: 'raw', value: '</div>' },
    ])

    assert.deepStrictEqual(result.templates[outerTemplateId], [
      { type: 'raw', value: '<div f-repeat="innerRepeat" >' },
      { type: 'repeat', value: 'innerRepeat', template: innerTemplateId },
      { type: 'raw', value: '</div>' },
    ])

    assert.deepStrictEqual(result.templates[innerTemplateId], [
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
})
