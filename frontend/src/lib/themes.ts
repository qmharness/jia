export interface DarkTokens {
  bgPrimary: string;
  bgSecondary: string;
  bgTertiary: string;
  textPrimary: string;
  textSecondary: string;
  textTertiary: string;
  border: string;
}

const DARK: DarkTokens = {
  bgPrimary: '#1a1b2e',
  bgSecondary: '#222438',
  bgTertiary: '#2d2f45',
  textPrimary: '#e8e9f0',
  textSecondary: '#9ca0b4',
  textTertiary: '#6b7084',
  border: '#2d2f45',
};

export interface ThemeDef {
  id: string;
  label: string;
  accent: string;
  hover: string;
  light: string;
  source: string;
  category: string;
  mode: 'light' | 'dark';
  dark?: DarkTokens;
}

export const THEMES: ThemeDef[] = [
  // ── 十天干 (10) ─────────────────────────────────────
  { id: 'jiamu',    label: '甲木青', accent: '#0f766e', hover: '#0d5e57', light: '#f0fdfa', category: '十天干', mode: 'light', source: '东方阳木，参天之势，万物始生' },
  { id: 'yimu',     label: '乙木柔', accent: '#5b8c1a', hover: '#4a7315', light: '#f7fee7', category: '十天干', mode: 'light', source: '东方阴木，藤萝屈伸，柔而不断' },
  { id: 'binghuo',  label: '丙火炽', accent: '#e11d48', hover: '#be123d', light: '#fff1f2', category: '十天干', mode: 'light', source: '南方阳火，烈日当空，光耀万物' },
  { id: 'dinghuo',  label: '丁火微', accent: '#c2410c', hover: '#9a3412', light: '#fff7ed', category: '十天干', mode: 'light', source: '南方阴火，烛火摇红，暗室微明' },
  { id: 'wutu',     label: '戊土厚', accent: '#8c7a6b', hover: '#6e5f52', light: '#faf9f7', category: '十天干', mode: 'light', source: '中央阳土，城垣厚重，承载四象' },
  { id: 'jitu',     label: '己土润', accent: '#a47148', hover: '#825c36', light: '#fef9f3', category: '十天干', mode: 'light', source: '中央阴土，田园湿润，孕育稼穑' },
  { id: 'gengjin',  label: '庚金锋', accent: '#636674', hover: '#4b4e5c', light: '#fafafa', category: '十天干', mode: 'light', source: '西方阳金，刀斧肃杀，锋从磨砺' },
  { id: 'xinjin',   label: '辛金璨', accent: '#7c3aed', hover: '#6d28d9', light: '#f5f3ff', category: '十天干', mode: 'light', source: '西方阴金，珠玉琳琅，清辉自照' },
  { id: 'renshui',  label: '壬水阔', accent: '#0369a1', hover: '#075985', light: '#f0f9ff', category: '十天干', mode: 'light', source: '北方阳水，江河奔涌，浩荡不息' },
  { id: 'guishui',  label: '癸水微', accent: '#0d9488', hover: '#0f766e', light: '#f0fdfa', category: '十天干', mode: 'light', source: '北方阴水，雨露潜润，无声化物' },

  // ── 九宫八卦 (9) ────────────────────────────────────
  { id: 'kanshui',  label: '坎水玄', accent: '#1a2740', hover: '#0f1b2d', light: '#f4f6fa', category: '九宫八卦', mode: 'dark', dark: DARK, source: '坎一白水，北方陷险，藏深渊于玄默' },
  { id: 'kunyu',    label: '坤玉黄', accent: '#854d0e', hover: '#6b3d0a', light: '#fefce8', category: '九宫八卦', mode: 'light', source: '坤二黑土，西南厚德，载万物而不言' },
  { id: 'zhenlei',  label: '震雷碧', accent: '#166534', hover: '#14532d', light: '#f0fdf4', category: '九宫八卦', mode: 'light', source: '震三碧木，东方雷动，蛰虫惊而出走' },
  { id: 'xunfeng',  label: '巽风翠', accent: '#15803d', hover: '#166534', light: '#dcfce7', category: '九宫八卦', mode: 'light', source: '巽四绿木，东南风入，顺逆皆从容' },
  { id: 'qilin',    label: '麒麟玉', accent: '#059669', hover: '#047857', light: '#ecfdf5', category: '九宫八卦', mode: 'light', source: '中五黄土，麒麟踞中，温润而泽四方' },
  { id: 'qiandao',  label: '乾道银', accent: '#5c6b7d', hover: '#455366', light: '#f8fafc', category: '九宫八卦', mode: 'light', source: '乾六白金，西北天行，健而不息' },
  { id: 'duize',    label: '兑泽靛', accent: '#4338ca', hover: '#3730a3', light: '#eef2ff', category: '九宫八卦', mode: 'light', source: '兑七赤金，西方泽悦，言出而众和' },
  { id: 'genzhi',   label: '艮止赭', accent: '#92400e', hover: '#78350f', light: '#fef3c7', category: '九宫八卦', mode: 'light', source: '艮八白土，东北山止，知止而后定' },
  { id: 'lihuo',    label: '离火赤', accent: '#b91c1c', hover: '#991b1b', light: '#fff1f2', category: '九宫八卦', mode: 'light', source: '离九紫火，南方明丽，丽乎天而文乎人' },

  // ── 九星 (9) ────────────────────────────────────────
  { id: 'tianpeng', label: '天蓬星', accent: '#1e3a8a', hover: '#172d6b', light: '#eff6ff', category: '九星', mode: 'dark', dark: DARK, source: '坎宫水宿，智勇深沉，幽波藏蛟龙' },
  { id: 'tianrui',  label: '天芮星', accent: '#6d4628', hover: '#55361e', light: '#fdf6f0', category: '九星', mode: 'light', source: '坤宫土宿，厚德载物，病坊亦慈航' },
  { id: 'tianchong',label: '天冲星', accent: '#2d7d3f', hover: '#226430', light: '#f0fdf4', category: '九星', mode: 'light', source: '震宫木宿，破土凌霄，一击千钧力' },
  { id: 'tianfu',   label: '天辅星', accent: '#3b82bf', hover: '#2d6a9e', light: '#eff6ff', category: '九星', mode: 'light', source: '巽宫木宿，文昌司命，笔落惊风雨' },
  { id: 'tianqin',  label: '天禽星', accent: '#9c8b7a', hover: '#7d6e5e', light: '#faf8f5', category: '九星', mode: 'light', source: '中宫土宿，飞步九宫，执中而御四方' },
  { id: 'tianxin',  label: '天心星', accent: '#a8552c', hover: '#884323', light: '#fff7ed', category: '九星', mode: 'light', source: '乾宫金宿，医卜星象，慧心通造化' },
  { id: 'tianzhu',  label: '天柱星', accent: '#5e5a57', hover: '#474441', light: '#fafaf9', category: '九星', mode: 'light', source: '兑宫金宿，擎天立极，风雨不动安如山' },
  { id: 'tianren',  label: '天任星', accent: '#9a3412', hover: '#7c2d12', light: '#fdf5ef', category: '九星', mode: 'light', source: '艮宫土宿，肩负重任，厚土无言自成蹊' },
  { id: 'tianying', label: '天英星', accent: '#dc2626', hover: '#b91c1c', light: '#fef2f2', category: '九星', mode: 'light', source: '离宫火宿，英华外发，烈烈其光不可掩' },

  // ── 八神 (8) ────────────────────────────────────────
  { id: 'zhiyin',   label: '值符金', accent: '#8b5a2b', hover: '#6d4522', light: '#fdf6f0', category: '八神', mode: 'light', source: '值符之首，天乙在前，六甲元首号至尊' },
  { id: 'tengshe',  label: '螣蛇诡', accent: '#86198f', hover: '#701a75', light: '#faf5ff', category: '八神', mode: 'dark', dark: DARK, source: '螣蛇虚诈，盘曲如雾，虚中有实变无常' },
  { id: 'taiyin',   label: '太阴幽', accent: '#4c1d95', hover: '#3b0764', light: '#f5f3ff', category: '八神', mode: 'dark', dark: DARK, source: '太阴荫庇，暗室逢灯，幽深之处自有光' },
  { id: 'liuhe',    label: '六合柔', accent: '#be185d', hover: '#9d174d', light: '#fdf2f8', category: '八神', mode: 'light', source: '六合和合，婚嫁媒妁，刚柔相济两不伤' },
  { id: 'baihu-s',  label: '白虎神', accent: '#881337', hover: '#6b0f2c', light: '#fdf2f8', category: '八神', mode: 'light', source: '白虎威猛，金气肃杀，凛凛不可犯其锋' },
  { id: 'xuanwu-s', label: '玄武神', accent: '#1e3a5f', hover: '#162d4a', light: '#f0f4fa', category: '八神', mode: 'dark', dark: DARK, source: '玄武潜伏，玄冥司水，大智若愚深不测' },
  { id: 'jiudi',    label: '九地沉', accent: '#44403c', hover: '#292524', light: '#fafaf9', category: '八神', mode: 'dark', dark: DARK, source: '九地深邃，藏锋守拙，厚积薄发待其时' },
  { id: 'jiutian',  label: '九天扬', accent: '#0474b5', hover: '#036091', light: '#f0f9ff', category: '八神', mode: 'light', source: '九天高扬，鹏抟九万，扶摇直上入青云' },
];

export function getTheme(id: string): ThemeDef {
  return THEMES.find(t => t.id === id) ?? THEMES[0];
}

// ── Dark mode utilities ──────────────────────────────────

const DARK_OVERRIDES: Record<string, string> = {
  '--bg-primary':    '#1a1b2e',
  '--bg-secondary':  '#222438',
  '--bg-tertiary':   '#2d2f45',
  '--bg-sidebar':    'rgba(26, 27, 46, 0.7)',
  '--text-primary':  '#e8e9f0',
  '--text-secondary':'#9ca0b4',
  '--text-tertiary': '#6b7084',
  '--border':        '#2d2f45',
};

const LIGHT_DEFAULTS: Record<string, string> = {
  '--bg-primary':    '#ffffff',
  '--bg-secondary':  '#f5f5f7',
  '--bg-tertiary':   '#e8e8ed',
  '--bg-sidebar':    'rgba(246, 246, 248, 0.7)',
  '--text-primary':  '#1d1d1f',
  '--text-secondary':'#6e6e73',
  '--text-tertiary': '#aeaeb2',
  '--border':        '#e5e5ea',
};

export function applyDarkMode(enabled: boolean) {
  const root = document.documentElement;
  const overrides = enabled ? DARK_OVERRIDES : LIGHT_DEFAULTS;
  if (enabled) {
    root.classList.add('dark');
  } else {
    root.classList.remove('dark');
  }
  for (const [key, val] of Object.entries(overrides)) {
    root.style.setProperty(key, val);
  }
}
