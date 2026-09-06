<template>
  <div class="app">
    <HeaderBar title="Vue Codebase" />
    <button @click="greet">Click</button>
  </div>
</template>

<script setup lang="ts">
import HeaderBar from './HeaderBar.vue';

function greet() {
  console.log("Welcome to Vue recall");
}
</script>
