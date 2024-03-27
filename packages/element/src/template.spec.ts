import assert from 'assert'
import { processTemplate } from './template';

describe('processTemplate', () => {
  it('should return the same string when there are no replacements', async () => {
    const template = 'Hello, world!';
    const result = await processTemplate(template, {});
    assert.equal(result, 'Hello, world!');
  });

  it('should replace keys with their corresponding values', async () => {
    const template = 'Hello, {{name}}!';
    const result = await processTemplate(template, { name: 'John' });
    assert.equal(result, 'Hello, John!');
  });

  it('should replace multiple keys with their corresponding values', async () => {
    const template = 'Hello, {{name.first}} {{name.last}}, sincerly, {{name.first}}!';
    const result = await processTemplate(template, { name: { first: 'John', last: 'Doe' } });
    assert.equal(result, 'Hello, John Doe, sincerly, John!');
  });

  it('should throw an error when a key is not found', async () => {
    const template = 'Hello, {{name}}!';
    try {
      await processTemplate(template, {});
      assert.fail('Expected processTemplate to throw an error, but it did not');
    } catch (err) {
      assert.equal(err.message, 'Replacement key "name" not found');
    }
  });

  it('should throw an error when an expression is not closed', async () => {
    const template = 'Hello, {{name!';
    try {
      await processTemplate(template, { name: 'John' });
      assert.fail('Expected processTemplate to throw an error, but it did not');
    } catch (err) {
      assert.equal(err.message, 'Template expression not closed');
    }
  });
});