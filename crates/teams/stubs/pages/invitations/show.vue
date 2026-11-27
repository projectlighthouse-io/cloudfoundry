<script setup lang="ts">
import { Link } from '@inertiajs/vue3'

defineProps<{ email: string; problem: string }>()
</script>

<template>
  <main>
    <h1>Invitation</h1>
    <p>{{ problem }}</p>
    <p>You are logged in as {{ email }}.</p>
    <p><Link href="/teams">Your teams</Link></p>
  </main>
</template>
