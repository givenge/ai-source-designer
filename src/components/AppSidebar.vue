<template>
  <aside class="shell-sidebar">
    <div class="brand">
      <h1>AI Source Designer</h1>
    </div>

    <nav class="shell-nav">
      <RouterLink
        to="/tasks"
        class="shell-nav-link"
        active-class="is-active"
      >
        任务台
      </RouterLink>
      <RouterLink
        to="/settings"
        class="shell-nav-link"
        active-class="is-active"
      >
        设置
      </RouterLink>
    </nav>
  </aside>
</template>
