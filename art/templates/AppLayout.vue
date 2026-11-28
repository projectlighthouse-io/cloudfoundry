<script setup lang="ts">
import { Head, Link } from '@inertiajs/vue3'
import UserMenu from '../components/UserMenu.vue'

// `art new` wrote the app name into the brand and footer.
// `wide` keeps the full rail width (landing pages); signed-in pages use a narrower column.
defineProps<{ title: string; user?: string | null; wide?: boolean }>()
</script>

<template>
  <Head :title="title" />
  <div class="page" :class="{ 'page-app': !wide }">
    <a class="skip" href="#main">Skip to content</a>
    <header class="nav">
      <div class="rail-box nav-in">
        <Link class="brand" href="/">
          <svg class="logo" viewBox="0 0 24 24" aria-hidden="true"><rect width="24" height="24" rx="6" /><circle cx="12" cy="12" r="4" fill="#4db6a0" /></svg>
          <span>{name}</span>
        </Link>
        <div class="nav-r">
          <UserMenu v-if="user" :email="user" />
        </div>
      </div>
    </header>
    <main id="main">
      <div class="rail-box"><slot /></div>
    </main>
    <footer class="foot">
      <div class="rail-box foot-legal mono">
        <span>{name}</span>
      </div>
    </footer>
  </div>
</template>
