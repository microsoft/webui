

import { run, bench, boxplot } from 'mitata'
import { createBuildTimeProtocol } from './replay_protocol.js'

const htmlContent = `
<div f-repeat="users">
  <custom-user f-signal="userName">
    <custom-header f-when="user.isActive">
      <h1 f-signal="userName">Default Name</h1>
      <p f-signal="userStatus">Status: Active</p>
    </custom-header>
    <custom-content>
      <ul f-repeat="user.items">
        <li f-signal="itemName">Default Item</li>
      </ul>
      <div f-when="user.hasDetails">
        <p f-signal="userDetails">User has additional details.</p>
      </div>
    </custom-content>
    <custom-footer>
      <button f-signal="actionLabel">Click Me</button>
      <span f-when="user.isPremium">Premium Member</span>
    </custom-footer>
  </custom-user>
</div>
`
const templates = {
  'custom-user': {
    template: `
        <section class="user-profile">
          <slot></slot>
        </section>
      `,
  },
  'custom-header': {
    template: `
        <header class="user-header">
          <slot></slot>
        </header>
      `,
  },
  'custom-content': {
    template: `
        <div class="user-content">
          <slot></slot>
        </div>
      `,
  },
  'custom-footer': {
    template: `
        <footer class="user-footer">
          <slot></slot>
        </footer>
      `,
  },
  'custom-button': {
    template: `
        <button class="action-button">{{actionLabel}}</button>
      `,
  },
}

bench('warmup', () => createBuildTimeProtocol(htmlContent, templates))

boxplot(() => {
  bench('create', () => createBuildTimeProtocol(htmlContent, templates))
})


await run({
  throw: false,
  filter: /.*/,
  colors: true,
  format: 'mitata',
})
