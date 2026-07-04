<div class="flex items-center justify-center p-12">
  <div class="relative w-14 h-14">
    <span class="absolute inset-0 flex items-center justify-center text-3xl font-semibold opacity-30"
          style="color: var(--text-tertiary)">甲</span>
    <div
      class="absolute inset-0 rounded-xl overflow-hidden"
      style="background: linear-gradient(112deg, transparent 22%, rgba(255,255,255,0.06) 38%, rgba(255,255,255,0.4) 49%, rgba(255,255,255,0.08) 56%, transparent 74%);
             background-size: 220% 100%;
             background-position: 140% 0;
             background-repeat: no-repeat;
             animation: shimmer 2.2s cubic-bezier(0.4, 0, 0.2, 1) infinite;"
    ></div>
  </div>
</div>

<style>
  @keyframes shimmer {
    0% { background-position: 140% 0; }
    100% { background-position: -120% 0; }
  }
</style>
