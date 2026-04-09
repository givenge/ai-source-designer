<template>
  <aside class="shell-sidebar">
    <div class="brand">
      <p class="brand-kicker">
        Reader-Rust Compatible
      </p>
      <h1>AI Source Designer</h1>
      <p class="brand-copy">
        先采样，再生成，再验证。把模型输出压回到可证明、可导出的书源 JSON。
      </p>
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

    <div class="sidebar-footnote">
      <p>v1 闭环：fixture 优先，公开站点仅做手工冒烟。</p>
    </div>
  </aside>
</template>
