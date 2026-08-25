// 管理面板入口：Vue 3 + Element Plus（暗色主题，简体中文）。
// 与外壳的通信全部走 Tauri 命令（window.__TAURI__.core，见 bridge.js）。
import { createApp } from 'vue';
import ElementPlus from 'element-plus';
import zhCn from 'element-plus/es/locale/lang/zh-cn';
import 'element-plus/dist/index.css';
import 'element-plus/theme-chalk/dark/css-vars.css';
import './theme.css';
import App from './App.vue';

createApp(App).use(ElementPlus, { locale: zhCn }).mount('#app');
