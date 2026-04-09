import { createRouter, createWebHistory } from 'vue-router';
import SettingsPage from '@/pages/SettingsPage.vue';
import TaskDetailPage from '@/pages/TaskDetailPage.vue';
import TasksPage from '@/pages/TasksPage.vue';

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: '/',
      redirect: '/tasks',
    },
    {
      path: '/tasks',
      component: TasksPage,
    },
    {
      path: '/tasks/:taskId',
      component: TaskDetailPage,
      props: true,
    },
    {
      path: '/settings',
      component: SettingsPage,
    },
  ],
});

export default router;
