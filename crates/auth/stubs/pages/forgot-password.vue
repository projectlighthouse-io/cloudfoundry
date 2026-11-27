<script setup lang="ts">
import { Link, useForm } from '@inertiajs/vue3'

defineProps<{ status: string | null }>()

const form = useForm({ email: '' })
</script>

<template>
  <main>
    <h1>Forgot your password?</h1>
    <p>Enter your email and we will send you a link to choose a new one.</p>
    <p v-if="status">{{ status }}</p>
    <form @submit.prevent="form.post('/forgot-password')">
      <label>
        Email
        <input v-model="form.email" type="email" autocomplete="username" required />
      </label>
      <p v-if="form.errors.email">{{ form.errors.email }}</p>

      <button type="submit" :disabled="form.processing">Email reset link</button>
    </form>
    <p><Link href="/login">Back to log in</Link></p>
  </main>
</template>
