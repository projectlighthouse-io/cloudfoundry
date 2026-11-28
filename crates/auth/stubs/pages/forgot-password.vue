<script setup lang="ts">
import { Link, useForm } from '@inertiajs/vue3'
import Field from '../../components/Field.vue'
import AuthLayout from '../../layouts/AuthLayout.vue'

defineProps<{ status: string | null }>()

const form = useForm({ email: '' })
</script>

<template>
  <AuthLayout title="Forgot your password?" subtitle="Enter your email and we will send you a link to choose a new one.">
    <p v-if="status" class="status" role="status">{{ status }}</p>
    <form @submit.prevent="form.post('/forgot-password')">
      <Field v-model="form.email" label="Email" type="email" autocomplete="username" :error="form.errors.email" autofocus />
      <button class="btn btn-ink btn-lg btn-block" type="submit" :disabled="form.processing">Email reset link</button>
    </form>
    <template #foot><Link href="/login">Back to log in</Link></template>
  </AuthLayout>
</template>
