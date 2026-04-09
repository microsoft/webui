// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import DefaultTheme from 'vitepress/theme'
import { useRoute } from 'vitepress'
import { watch, nextTick } from 'vue'
import './custom.css'

export default {
  extends: DefaultTheme,
  setup() {
    const route = useRoute()
    watch(() => route.path, () => {
      nextTick(() => {
        document.querySelector('#VPContent')?.scrollTo({ top: 0 })
      })
    })
  },
}
