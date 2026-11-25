/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  theme: {
    extend: {
      colors: {
        night: {
          950: '#04070d',
          900: '#060a12',
          850: '#080e18',
          800: '#0b1220',
          750: '#0e1726',
          700: '#122032',
        },
        mint: {
          200: '#c3f9f4',
          300: '#8ff3e2',
          400: '#6ee7ff',
          500: '#34d399',
          600: '#22b98a',
        },
        amber: {
          300: '#fcd34d',
          400: '#fbbf24',
          500: '#f59e0b',
        },
        coral: {
          300: '#fda4af',
          400: '#fb7185',
          500: '#e11d48',
        },
        ink: {
          50: '#f2f7fb',
          100: '#dce7f2',
          200: '#b9c9d9',
          300: '#8da3b8',
          400: '#6e8296',
          500: '#4d5f72',
          600: '#364655',
        },
      },
      fontFamily: {
        sans: ['"Inter Variable"', 'Inter', 'system-ui', 'sans-serif'],
        display: ['"Outfit Variable"', 'Outfit', 'system-ui', 'sans-serif'],
        mono: ['"JetBrains Mono"', 'ui-monospace', 'SFMono-Regular', 'Menlo', 'monospace'],
      },
      borderRadius: {
        xl2: '1.25rem',
        '4xl': '2rem',
      },
      boxShadow: {
        glass: 'inset 0 1px 0 0 rgba(255,255,255,0.06), 0 24px 60px -30px rgba(0,0,0,0.9)',
        'glass-strong': 'inset 0 1px 0 0 rgba(255,255,255,0.09), 0 40px 80px -40px rgba(0,0,0,0.95)',
        glow: '0 0 0 1px rgba(110,231,255,0.25), 0 0 40px -8px rgba(110,231,255,0.35)',
        'glow-amber': '0 0 0 1px rgba(251,191,36,0.3), 0 0 36px -10px rgba(251,191,36,0.4)',
        'glow-coral': '0 0 0 1px rgba(251,113,133,0.3), 0 0 36px -10px rgba(251,113,133,0.4)',
      },
      backgroundImage: {
        'mint-gradient': 'linear-gradient(135deg, #6ee7ff 0%, #34d399 100%)',
        'amber-gradient': 'linear-gradient(135deg, #fcd34d 0%, #f59e0b 100%)',
        'coral-gradient': 'linear-gradient(135deg, #fda4af 0%, #fb7185 100%)',
      },
      transitionTimingFunction: {
        smooth: 'cubic-bezier(0.22, 0.61, 0.36, 1)',
      },
      animation: {
        'fade-up': 'fade-up 0.5s cubic-bezier(0.22, 0.61, 0.36, 1) both',
        'fade-in': 'fade-in 0.3s ease both',
        shimmer: 'shimmer 2.4s linear infinite',
      },
      keyframes: {
        'fade-up': {
          from: { opacity: '0', transform: 'translateY(10px)' },
          to: { opacity: '1', transform: 'translateY(0)' },
        },
        'fade-in': {
          from: { opacity: '0' },
          to: { opacity: '1' },
        },
        shimmer: {
          from: { backgroundPosition: '200% 0' },
          to: { backgroundPosition: '-200% 0' },
        },
      },
    },
  },
  plugins: [],
};
