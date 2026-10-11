// The worker of 132-worker-canvas.html (M31): draws on the canvas it is
// given and says so; what it drew stays in the page's canvas.
onmessage = function (e) {
  var context = e.data.canvas.getContext('2d');
  context.fillStyle = '#1e3a8a';
  context.fillRect(0, 0, 320, 200);
  var gradient = context.createLinearGradient(20, 0, 300, 0);
  gradient.addColorStop(0, '#f59e0b');
  gradient.addColorStop(1, '#ef4444');
  context.fillStyle = gradient;
  context.fillRect(20, 20, 280, 40);
  context.fillStyle = '#10b981';
  context.beginPath();
  context.arc(80, 130, 40, 0, Math.PI * 2);
  context.fill();
  context.strokeStyle = '#ffffff';
  context.lineWidth = 6;
  context.lineCap = 'round';
  context.beginPath();
  context.moveTo(160, 100);
  context.lineTo(300, 180);
  context.stroke();
  context.fillStyle = 'rgba(255, 255, 255, 0.5)';
  context.fillRect(180, 80, 100, 60);
  // Said once a frame has shown what it drew: the reference browser
  // commits a worker's canvas at its next frame, not as it closes.
  requestAnimationFrame(function () {
    postMessage('drawn');
  });
};
