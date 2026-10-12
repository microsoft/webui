// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import * as ts from "./typescript-api.js";

interface Scope {
  readonly node: ts.Node;
  readonly start: number;
  readonly parent?: Scope;
  readonly functionScope: boolean;
  readonly names: Set<string>;
  readonly children: Scope[];
  readonly dynamic: boolean;
}

export class SourceScopes {
  readonly assigned = new Set<string>();
  private readonly root: Scope;

  constructor(source: ts.SourceFile, visit: (node: ts.Node) => void) {
    this.root = createScope(source, undefined, true);
    const stack = [{ node: source as ts.Node, scope: this.root }];
    const writes: Array<{ node: ts.Node; scope: Scope }> = [];
    while (stack.length > 0) {
      const current = stack.pop()!;
      const { node } = current;
      let { scope } = current;
      declareOuterName(node, scope);
      if (node !== source && startsScope(node)) {
        const child = createScope(node, scope, isFunction(node) ||
          isFunction(scope.node) && scope.node.body === node ||
          node.kind === ts.SyntaxKind.ClassStaticBlockDeclaration);
        scope.children.push(child);
        scope = child;
        declareScopeNames(node, scope);
      }
      if (ts.isVariableDeclarationList(node)) declareVariables(node, scope);
      const target = assignmentTarget(node);
      if (target) writes.push({ node: target, scope });
      visit(node);
      const children: ts.Node[] = [];
      ts.forEachChild(node, (child) => {
        children.push(child);
      });
      for (let index = children.length - 1; index >= 0; index--) {
        const child = children[index]!;
        const outerEvaluation = isFunction(node) &&
          (ts.isComputedPropertyName(child) || ts.isDecorator(child));
        stack.push({ node: child, scope: outerEvaluation ? current.scope : scope });
      }
    }
    const scopes = [this.root];
    while (scopes.length > 0) {
      const scope = scopes.pop()!;
      scope.children.sort((left, right) => left.start - right.start);
      for (const child of scope.children) scopes.push(child);
    }
    // Declarations may follow a write or reference in source order.
    for (const { node, scope } of writes) {
      targetNames(node, (name) => {
        let owner: Scope | undefined = scope;
        while (owner?.parent && !owner.names.has(name)) owner = owner.parent;
        if (owner === this.root) this.assigned.add(name);
      });
    }
  }

  shadows(node: ts.Node, name: string): boolean {
    let scope = this.root;
    while (true) {
      if (scope.parent && (scope.dynamic || scope.names.has(name))) return true;
      const children = scope.children;
      let low = 0;
      let high = children.length;
      while (low < high) {
        const middle = (low + high) >>> 1;
        if (children[middle]!.start <= node.pos) low = middle + 1;
        else high = middle;
      }
      const child = children[low - 1];
      if (!child || node.end > child.node.end) return false;
      scope = child;
    }
  }
}

function createScope(node: ts.Node, parent: Scope | undefined, functionScope: boolean): Scope {
  return {
    node, parent, functionScope, names: new Set(), children: [],
    start: isFunction(node) ? node.parameters[0]?.pos ?? node.body?.pos ?? node.end : node.pos,
    dynamic: ts.isWithStatement(node),
  };
}

function startsScope(node: ts.Node): boolean {
  return isFunction(node) ||
    ts.isClassDeclaration(node) || ts.isClassExpression(node) ||
    ts.isBlock(node) || ts.isCaseBlock(node) || ts.isCatchClause(node) ||
    ts.isForStatement(node) || ts.isForInStatement(node) || ts.isForOfStatement(node) ||
    ts.isWithStatement(node) || node.kind === ts.SyntaxKind.ClassStaticBlockDeclaration;
}

function isFunction(node: ts.Node): node is
  ts.FunctionDeclaration | ts.FunctionExpression | ts.ArrowFunction |
  ts.MethodDeclaration | ts.ConstructorDeclaration |
  ts.GetAccessorDeclaration | ts.SetAccessorDeclaration {
  return ts.isFunctionDeclaration(node) || ts.isFunctionExpression(node) ||
    ts.isArrowFunction(node) || ts.isMethodDeclaration(node) ||
    ts.isConstructorDeclaration(node) || ts.isGetAccessorDeclaration(node) ||
    ts.isSetAccessorDeclaration(node);
}

function declareOuterName(node: ts.Node, scope: Scope): void {
  if ((ts.isClassDeclaration(node) || ts.isFunctionDeclaration(node)) && node.name) {
    scope.names.add(node.name.text);
  }
}

function declareScopeNames(node: ts.Node, scope: Scope): void {
  if (isFunction(node)) {
    for (const parameter of node.parameters) {
      targetNames(parameter.name, (name) => scope.names.add(name));
    }
  }
  if (
    (ts.isClassDeclaration(node) || ts.isClassExpression(node) ||
      ts.isFunctionExpression(node)) && node.name
  ) {
    scope.names.add(node.name.text);
  }
  if (ts.isCatchClause(node) && node.variableDeclaration) {
    targetNames(node.variableDeclaration.name, (name) => scope.names.add(name));
  }
}

function declareVariables(node: ts.VariableDeclarationList, scope: Scope): void {
  let owner = scope;
  if (!(node.flags & ts.NodeFlags.BlockScoped)) {
    while (owner.parent && !owner.functionScope) owner = owner.parent;
  }
  for (const declaration of node.declarations) {
    targetNames(declaration.name, (name) => owner.names.add(name));
  }
}

function assignmentTarget(node: ts.Node): ts.Node | undefined {
  if (
    ts.isBinaryExpression(node) &&
    node.operatorToken.kind >= ts.SyntaxKind.FirstAssignment &&
    node.operatorToken.kind <= ts.SyntaxKind.LastAssignment
  ) {
    return node.left;
  }
  if (
    (ts.isPrefixUnaryExpression(node) || ts.isPostfixUnaryExpression(node)) &&
    (node.operator === ts.SyntaxKind.PlusPlusToken ||
      node.operator === ts.SyntaxKind.MinusMinusToken)
  ) {
    return node.operand;
  }
  if (ts.isForInStatement(node) || ts.isForOfStatement(node)) return node.initializer;
  return undefined;
}

function targetNames(node: ts.Node, visit: (name: string) => void): void {
  const stack = [node];
  while (stack.length > 0) {
    const current = stack.pop()!;
    if (ts.isIdentifier(current)) {
      visit(current.text);
    } else if (ts.isVariableDeclarationList(current)) {
      for (const declaration of current.declarations) stack.push(declaration.name);
    } else if (ts.isBindingElement(current)) {
      if (current.name) stack.push(current.name);
    } else if (ts.isObjectBindingPattern(current) || ts.isArrayBindingPattern(current)) {
      for (const element of current.elements) stack.push(element);
    } else if (ts.isObjectLiteralExpression(current)) {
      for (const property of current.properties) stack.push(property);
    } else if (ts.isArrayLiteralExpression(current)) {
      for (const element of current.elements) stack.push(element);
    } else if (ts.isPropertyAssignment(current)) {
      stack.push(current.initializer);
    } else if (ts.isShorthandPropertyAssignment(current)) {
      stack.push(current.name);
    } else if (ts.isBinaryExpression(current)) {
      stack.push(current.left);
    } else if (
      ts.isSpreadAssignment(current) || ts.isSpreadElement(current) ||
      ts.isParenthesizedExpression(current) || ts.isAsExpression(current) ||
      ts.isTypeAssertionExpression(current) || ts.isNonNullExpression(current) ||
      ts.isSatisfiesExpression(current)
    ) {
      stack.push(current.expression);
    }
  }
}
