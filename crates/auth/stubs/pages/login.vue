<script setup lang="ts">
import { Link, useForm } from '@inertiajs/vue3'

const form = useForm({ email: '', password: '', remember: false })
</script>

<template>
  <main>
    <h1>Log in</h1>
    <form @submit.prevent="form.post('/login')">
      <label>
        Email
        <input v-model="form.email" type="email" autocomplete="username" required />
      </label>
      <p v-if="form.errors.email">{{ form.errors.email }}</p>

      <label>
        Password
        <input v-model="form.password" type="password" autocomplete="current-password" required />
      </label>
      <p v-if="form.errors.password">{{ form.errors.password }}</p>

      <label>
        <input v-model="form.remember" type="checkbox" />
        Remember me
      </label>

      <button type="submit" :disabled="form.processing">Log in</button>
    </form>
    <p>No account yet? <Link href="/register">Register</Link></p>
  </main>
</template>
