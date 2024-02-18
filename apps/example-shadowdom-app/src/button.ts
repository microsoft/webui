class AppButton extends HTMLElement {
  constructor() {
    super()
    this.attachShadow({ mode: 'open' })
  }

  connectedCallback() {
    this.shadowRoot!.innerHTML = `
      <style>
        :host {
          --background-color: #007bff;
          --text-color: #ffffff;
        }

        @media (prefers-color-scheme: dark) {
          :host {
            --background-color: #0066cc;
            --text-color: #ffffff;
          }
        }

        button {
          background-color: var(--background-color);
          color: var(--text-color);
          border: none;
          padding: 10px 20px;
          text-align: center;
          text-decoration: none;
          display: inline-block;
          font-size: 16px;
          margin: 4px 2px;
          cursor: pointer;
        }
      </style>
      <button><slot></slot></button>
    `
  }
}

customElements.define('app-button', AppButton)
