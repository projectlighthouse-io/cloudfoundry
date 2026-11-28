<script setup lang="ts">
import AppLayout from './AppLayout.vue'

defineProps<{ title: string; subtitle?: string }>()
</script>

<template>
  <AppLayout :title="title">
    <section class="auth">
      <div class="hero-tex" aria-hidden="true" />
      <div class="card">
        <h1>{{ title }}</h1>
        <p v-if="subtitle" class="sub">{{ subtitle }}</p>
        <slot />
        <div v-if="$slots.foot" class="card-foot"><slot name="foot" /></div>
      </div>
    </section>
  </AppLayout>
</template>
