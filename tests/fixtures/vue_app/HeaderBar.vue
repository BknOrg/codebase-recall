<template>
  <header>
    <h1>{{ title }}</h1>
  </header>
</template>

<script setup lang="ts">
export interface HeaderProps {
  title: string;
}
</script>
