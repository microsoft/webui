---
layout: page
title: Playground
---

<script setup>
import { defineAsyncComponent } from 'vue'
const Playground = defineAsyncComponent(() => import('../.vitepress/components/Playground.vue'))
</script>

<ClientOnly>
  <Playground />
</ClientOnly>

