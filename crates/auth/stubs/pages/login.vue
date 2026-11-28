<script setup lang="ts">
import { Link, useForm } from '@inertiajs/vue3'
import Field from '../../components/Field.vue'
import AuthLayout from '../../layouts/AuthLayout.vue'

const form = useForm({ email: '', password: '', remember: false })
const submit = () => form.post('/login', { onFinish: () => form.reset('password') })
</script>

<template>
  <AuthLayout title="Log in" subtitle="Welcome back. Enter your email and password.">
    <form @submit.prevent="submit">
      <Field v-model="form.email" label="Email" type="email" autocomplete="username" :error="form.errors.email" autofocus />
      <Field v-model="form.password" label="Password" type="password" autocomplete="current-password" :error="form.errors.password">
        <template #aside><Link class="link" href="/forgot-password" style="font-size: 13px">Forgot password?</Link></template>
      </Field>
      <label class="check"><input v-model="form.remember" type="checkbox" /> Remember me</label>
      <button class="btn btn-ink btn-lg btn-block" type="submit" :disabled="form.processing">Log in</button>
    </form>
    <template #foot>No account yet? <Link href="/register">Create one</Link></template>
  </AuthLayout>
</template>
