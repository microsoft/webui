import { html } from '@internal/fast-element'

export default html`
  <div class="header">
    <app-header></app-header>(<span f-signal="items.length">0</span>)
  </div>
  
  <p class="toast" f-when="toast" f-signal="toast"></p>
  
  <div>
    <app-button f-click="onClick">Add New</app-button>
    <app-button f-click="onClear">Clear All</app-button>
  </div>
  
  <div f-when="items.length > 5">You have more than 5 items!</div>
  
  <div class="items" f-repeat="items" w-component="app-item">
  </div>
  
  <div f-when="!items.length">No items to show</div>
`
