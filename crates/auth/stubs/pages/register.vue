<script setup lang="ts">
import { Link, useForm } from '@inertiajs/vue3'

const form = useForm({ email: '', password: '', password_confirmation: '' })
</script>

<template>
  <main>
    <h1>Create an account</h1>
    <form @submit.prevent="form.post('/register', { onFinish: () => form.reset('password', 'password_confirmation') })">
      <label>
        Email
        <input v-model="form.email" type="email" autocomplete="username" required />
      </label>
      <p v-if="form.errors.email">{{ form.errors.email }}</p>

      <label>
        Password
        <input v-model="form.password" type="password" autocomplete="new-password" required />
      </label>
      <p v-if="form.errors.password">{{ form.errors.password }}</p>

      <label>
        Confirm password
        <input v-model="form.password_confirmation" type="password" autocomplete="new-password" required />
      </label>

      <button type="submit" :disabled="form.processing">Register</button>
    </form>
    <p>Already registered? <Link href="/login">Log in</Link></p>
  </main>
</template>
