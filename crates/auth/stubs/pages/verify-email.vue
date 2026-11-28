<script setup lang="ts">
import type { VerifyEmailPage } from '../../types/VerifyEmailPage'
import { router } from '@inertiajs/vue3'
import AuthLayout from '../../layouts/AuthLayout.vue'

defineProps<VerifyEmailPage>()

const resend = () => router.post('/settings/email/verification-notification')
</script>

<template>
  <AuthLayout title="Verify your email" :subtitle="`We sent a link to ${email}. Open it to continue.`">
    <p v-if="status" class="flash flash-ok" role="status">{{ status }}</p>
    <p v-if="$page.props.errors?.email" class="flash flash-err" role="alert">{{ $page.props.errors.email }}</p>
    <button class="btn btn-ink btn-lg btn-block" type="button" @click="resend">Send a new link</button>
  </AuthLayout>
</template>
