---
description: Start a new WebUI app with HTML templates, server-provided data, and optional browser interactivity.
---

<span id="what-is-webui-framework"></span>

# What is WebUI?

WebUI builds web apps from HTML templates, CSS, and data. A compiler prepares
the templates before requests arrive. A native renderer fills them with your
backend's data to produce HTML. Add TypeScript where a component needs browser
behavior, such as responding to a button.

## Start here

These lessons assume you can edit HTML and run commands in a terminal.
Installation and the first three lessons use one `hello-webui` project.

| Step | Result |
| --- | --- |
| [Installation](/start/installation) | A project with a working local CLI |
| [Your first page](/start/first-page) | HTML rendered from a template and JSON data |
| [Add a component](/start/first-component) | A reusable count display with its own HTML and CSS |
| [Add interaction](/start/add-interactivity) | A button that updates the count in the browser |
| [Build a Todo app](/start/todo-app) | A separate app with lists and events between components |

Coming from React? The [comparison guide](/learn/react-comparison)
maps components, props, state, and events to WebUI. It uses the same counter as
the lessons above.

<span id="quick-overview"></span>
<span id="ready-to-try-it"></span>

To try bindings without creating a project, use the [playground](/playground/).
It edits HTML, CSS, and JSON data; it does not run component TypeScript.

## How WebUI works

<span id="1-build-compile-templates-to-binary"></span>
<span id="2-render-fill-in-state-data-from-any-backend"></span>
<span id="3-hydrate-interactive-islands-come-alive"></span>

The compiler reads your templates at build time. The server combines the
compiled result with request data. In the browser, interactive components
connect to the rendered HTML rather than replacing it.

The CLI handles compilation and HTTP serving for the local lessons. A
production backend supplies its own data and uses a
[host integration](/build/integrations/). Read
[How rendering works](/learn/how-it-works) for that division of work.

## Key concepts

Use these pages when you need more detail about a lesson:

| Reference | What it explains |
| --- | --- |
| [Template syntax](/learn/directives/) | Bindings, conditions, lists, and attributes |
| [Components](/learn/components/) | File naming, composition, and DOM boundaries |
| [State and data](/learn/state-management/) | Initial server data and browser updates |
| [Interactivity](/learn/interactivity) | Events, observable properties, and lifecycle |
| [CSS and themes](/learn/css-tokens) | Component styles and shared design tokens |
