# Reusable Local Fragments

Use `<fragment>` to declare reusable markup and `<render>` to insert it without
a wrapper element. A fragment belongs to one component or entry template. It is
not a component, does not create a custom element, and needs no JavaScript class.

## Declare and render

```html
<!-- item-list.html -->
<template>
  <h2>{{title}}</h2>
  <ul>
    <render fragment="list-items" scope="{{items}}" as="items"></render>
  </ul>

  <fragment name="list-items">
    <for each="item in items">
      <li>{{item.label}}</li>
    </for>
  </fragment>
</template>
```

Place declarations directly at the owning component's root, or directly inside
the entry's `<body>`. When an entry omits `<body>`, its top-level content
(inside `<html>`, if present) is the implicit body root. If the component has a sole root `<template>` wrapper,
put the declarations inside that wrapper, not beside it. This also applies to
root templates with a hydration policy or `shadowrootmode="open"`.

Do not nest a declaration inside an ordinary element, another directive,
another fragment, or inert/raw content. The declaration itself produces no
output. Calls may appear before it.

`<render>` inserts the body **once** at the callsite. Passing an array does not
repeat the body: use [`<for>`](./for) inside the fragment to iterate. The
`<render>` may contain only whitespace and comments, not rendered child
content. Its attributes select the fragment and, optionally, provide one input.
A self-closing call such as `<render fragment="heading" />` is also valid.

## Parameterless fragments

Omit both `scope` and `as` when the body only needs its owner's state or props:

```html
<template>
  <render fragment="heading"></render>

  <fragment name="heading">
    <h1>{{title}}</h1>
    <p>{{description}}</p>
  </fragment>
</template>
```

The body can have no nodes, one node, or multiple sibling nodes, including text.
Neither directive adds a DOM wrapper.

## Inputs and scope

`scope` and `as` must appear together:

```html
<render fragment="person-details" scope="{{selectedPerson}}" as="person"></render>
```

| Attribute | Where | Rule |
|---|---|---|
| `name` | `<fragment>` | Required static local name; ASCII letter or `_`, followed by ASCII letters, digits, `_`, or `-` |
| `fragment` | `<render>` | Required name of a declaration in the same owner |
| `scope` | `<render>` | One dotted input path, bare or double-braced; requires `as` |
| `as` | `<render>` | Alias beginning with an ASCII letter or `_`, followed by ASCII letters, digits, or `_`; requires `scope` |

Each attribute may occur only once. Other directive attributes are unsupported;
put element bindings and event handlers inside the fragment body.

The input is a single dotted state path. Both `scope="selectedPerson"` and
<code v-pre>scope="{{selectedPerson}}"</code> select the same input. Examples include
<code v-pre>{{selectedPerson}}</code>,
<code v-pre>{{item.children}}</code>, or
<code v-pre>{{items.length}}</code>. It is not an expression: calls, arithmetic,
ternaries, and bracket indexing are unsupported. Numeric array indexes such as
`items.0` are not supported; use a loop item instead. Arrays and strings support
`.length`. Each path segment follows the alias naming rule above. Triple-braced
inputs are not supported. String `.length` counts UTF-8 bytes, not characters;
see [State Management](../state-management/#path-resolution).

The input is evaluated at the callsite, so it can read the caller's loop item or
fragment alias. Inside the called body, lookup uses:

1. Loop variables introduced inside that body, from innermost to outermost.
2. The input alias named by `as`, if present.
3. The owning component's state and props, or the entry's state.

Caller loop variables and caller fragment aliases are not inherited. This
isolation also applies to parameterless calls. Pass any required caller-local
value explicitly.

An alias owns its entire root name. If `person` is the alias and its value has
no `name`, `person.name` stays missing; it does not fall back to a same-named
value in the owner's state.

A missing input path is an execution error with actionable help, not an empty
render. In contrast, `null`, `false`, `0`, `""`, `{}`, and `[]` are all valid
inputs. They still invoke the body once. The body's bindings, conditions, and
loops then apply their usual rules.

## Recursive trees

Fragments may call themselves or each other. Use data and conditions to stop
the traversal:

```html
<!-- tree-view.html -->
<template>
  <ul>
    <render fragment="tree-items" scope="{{items}}" as="items"></render>
  </ul>

  <fragment name="tree-items">
    <for each="item in items">
      <li>
        <span>{{item.label}}</span>
        <if condition="item.children.length">
          <ul>
            <render fragment="tree-items" scope="{{item.children}}" as="items"></render>
          </ul>
        </if>
      </li>
    </for>
  </fragment>
</template>
```

```json
{
  "items": [
    {
      "label": "Guides",
      "children": [
        { "label": "Getting started", "children": [] }
      ]
    },
    { "label": "Reference", "children": [] }
  ]
}
```

The nested call selects `item.children` before entering the next invocation.
That invocation receives a fresh `items` alias, not the caller's `item`.
An empty `children` array stops the branch.

Each server response or browser update permits at most **256 active fragment
calls** and **100,000 invocations**. The response budget is shared across
streaming `start`, `resume`, and `advance`; suspending does not reset it.
Exceeding either limit is an error, never silent truncation.

Streaming also retains its independent limit of 256 continuation frames.
Surrounding components, conditions, and other structural content consume those
frames, so a streamed response can reach that limit before 256 fragment calls.
Ordinary SSR and browser updates do not apply that streaming-frame limit.

## Hydration and updates

With the WebUI plugin, fragment content supports the owner's normal bindings,
events, state updates, and component lifecycle. A fragment does not create a
separate component instance or lifecycle callback.

Hydration keeps trusted server-rendered content rather than recreating it.
If an input is unavailable because its state was not sent to the browser, the
existing content stays in place until that state becomes available. This is
different from a known state value whose requested child path is missing:
that missing input is an error. See [Hydration](../hydration) and
[State Management](../state-management/).

## Streaming boundaries

A fragment may contain a [`<boundary>`](./boundary), subject to the same
placement rules as other template content. A boundary must not reach another
boundary through a fragment call, and a `<for>` body must not reach a boundary
through any sequence of fragment calls or components. Put the whole repeat
inside one boundary instead.

If multiple static callsites reach one boundary declaration, that boundary
requires a `key`, whether its owner is an entry or a component. See
[Boundary keys](./boundary#keys-for-multiple-static-callsites).

An active fragment invocation retains its selected input across streaming
suspension. Supplying new state to `resume` does not change that already
selected input. After hydration, unrelated owner-state updates and events keep
using that input; a later explicit update to its input dependency rebinds the
invocation normally.

## Authoring rules and support

- Use the lowercase `<fragment>` and `<render>` directive names.
- Names are case-sensitive, static, and local to the owning component or entry. Another
  component may use the same name, but cannot call this declaration.
- Dynamic fragment selection and cross-component fragment imports are not
  supported. Use a component when markup must be shared across owners.
- Duplicate declarations, unknown call targets, invalid inputs, misplaced
  declarations, and malformed directives fail the build. Unused declarations
  are still validated.
- Unreachable declarations do not add their markup or exclusively referenced
  component resources to the application output.
- Native SSR and `--plugin=webui` support these directives. FAST plugins reject
  them with an actionable build error.
- Rebuild the protocol and browser assets together when upgrading. Mixing old
  compiled artifacts with the new runtime is unsupported.
