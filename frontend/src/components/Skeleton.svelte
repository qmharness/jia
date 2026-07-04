<script lang="ts">
  let { width = '100%', height = '1rem', rounded = 'rounded-[6px]' }: {
    width?: string;
    height?: string;
    rounded?: string;
  } = $props();
</script>

<div
  class="animate-pulse bg-black/8 {rounded}"
  style="width: {width}; height: {height};"
></div>
