require('dotenv').config();
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

// 确保 Pi 的配置目录存在
const piConfigDir = path.join(__dirname, 'config', 'pi');
if (!fs.existsSync(piConfigDir)) {
  fs.mkdirSync(piConfigDir, { recursive: true });
  console.warn('Created missing Pi config directory:', piConfigDir);
  console.warn('Please ensure config/pi/models.json and auth.json are correctly set up.');
}

// 启动网关
const gateway = spawn('node', ['app.js'], {
  cwd: __dirname,
  stdio: 'inherit',
  shell: true,
});

// 等待网关启动（简单延时3秒，或可监听其输出）
setTimeout(() => {
  // 启动 Pi，并指定使用项目内的配置目录
  const pi = spawn('npx', ['pi'], {
    cwd: __dirname,
    stdio: 'inherit',
    shell: true,
    env: {
      ...process.env,
      PI_CODING_AGENT_DIR: piConfigDir,   // 关键：让 Pi 读取 config/pi 下的配置
    },
  });
  pi.on('error', (err) => {
    console.error('Failed to start Pi:', err);
    process.exit(1);
  });
}, 3000);

gateway.on('error', (err) => {
  console.error('Gateway failed to start:', err);
  process.exit(1);
});

// 处理 Ctrl+C 同时关闭两个进程
process.on('SIGINT', () => {
  console.log('\nShutting down...');
  gateway.kill('SIGINT');
  // Pi 也会随着父进程退出而退出，但显式 kill 更安全
  // 不过子进程可能未启动，忽略错误
  process.exit();
});