// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import * as ts from "./typescript-api.js";

export interface ClassFactory {
  readonly node: ts.ClassLikeDeclaration;
  readonly baseParameter: number;
}

export function analyzeClassFactory(
  declaration: ts.FunctionDeclaration
): ClassFactory | undefined {
  if (
    declaration.asteriskToken ||
    modifiers(declaration).some((node) => node.kind === ts.SyntaxKind.AsyncKeyword)
  ) {
    return undefined;
  }
  const statements = declaration.body?.statements;
  if (!statements) return undefined;
  const last = statements[statements.length - 1];
  if (!last || !ts.isReturnStatement(last) || !last.expression) return undefined;

  let node: ts.ClassLikeDeclaration;
  const first = statements[0];
  if (statements.length === 1 && ts.isClassExpression(last.expression)) {
    node = last.expression;
  } else if (
    statements.length === 2 &&
    first &&
    ts.isClassDeclaration(first) &&
    first.name &&
    ts.isIdentifier(last.expression) &&
    last.expression.text === first.name.text
  ) {
    node = first;
  } else {
    return undefined;
  }

  const names = new Set<string>();
  for (const parameter of declaration.parameters) {
    if (
      !ts.isIdentifier(parameter.name) ||
      parameter.name.text === "this" ||
      parameter.initializer ||
      parameter.dotDotDotToken ||
      names.has(parameter.name.text)
    ) {
      return undefined;
    }
    names.add(parameter.name.text);
  }

  const base = node.heritageClauses?.find(
    (clause) => clause.token === ts.SyntaxKind.ExtendsKeyword
  )?.types[0]?.expression;
  if (!base || !ts.isIdentifier(base)) return undefined;
  const baseParameter = declaration.parameters.findIndex(
    (parameter) => ts.isIdentifier(parameter.name) && parameter.name.text === base.text
  );
  if (baseParameter === -1) return undefined;
  if (node.name) {
    if (names.has(node.name.text)) return undefined;
    names.add(node.name.text);
  }

  if (
    modifiers(node).some((modifier) => modifier.kind === ts.SyntaxKind.Decorator) ||
    node.members.some((member) =>
      member.kind === ts.SyntaxKind.ClassStaticBlockDeclaration ||
      modifiers(member).some((modifier) => modifier.kind === ts.SyntaxKind.StaticKeyword)
    ) ||
    hasShadowedDecorator(node, names)
  ) {
    return undefined;
  }
  return { node, baseParameter };
}

function modifiers(node: ts.Node): readonly ts.ModifierLike[] {
  return (node as ts.Node & { readonly modifiers?: readonly ts.ModifierLike[] }).modifiers ?? [];
}

export function hasShadowedDecorator(
  node: ts.ClassLikeDeclaration,
  localNames: ReadonlySet<string>
): boolean {
  for (const member of node.members) {
    for (const modifier of modifiers(member)) {
      if (!ts.isDecorator(modifier)) continue;
      let target: ts.Expression = modifier.expression;
      if (ts.isCallExpression(target)) target = target.expression;
      while (ts.isPropertyAccessExpression(target)) target = target.expression;
      if (ts.isIdentifier(target) && localNames.has(target.text)) return true;
    }
  }
  return false;
}
