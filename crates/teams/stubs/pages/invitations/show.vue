<script setup lang="ts">
import type { InvitationPage } from '../../types/InvitationPage'
import { Link } from '@inertiajs/vue3'
import AppLayout from '../../layouts/AppLayout.vue'

defineProps<InvitationPage>()
</script>

<template>
  <AppLayout title="Invitation" :user="email">
    <header class="phead">
      <div class="phead-id">
        <div>
          <h1>This invitation cannot be used</h1>
          <p class="sub">{{ problem }}</p>
        </div>
      </div>
    </header>
    <div class="strip"><h2>Signed in as</h2><span class="meta">{{ email }}</span></div>
    <p class="empty-row">Ask whoever invited you for a new link, sent to this address. <Link class="link" href="/teams">Your teams</Link></p>
    <div class="tail" />
  </AppLayout>
</template>
