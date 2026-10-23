# 蝶梦 · UI 设计提示词

> 配套画布：[[蝶梦语音智能体.canvas]]
> 用途：把下面的提示词整段复制进图像生成模型，产出 UI 参考图。
> 整理日期：2026-10-23

---

## 0. 怎么用这份文件

| 项 | 建议 |
|---|---|
| 模型 | Nano Banana 2 / Midjourney v7 / Flux 1.1 Pro / 即梦 |
| 语言 | **英文提示词效果最稳**，中文只作理解参考 |
| 比例 | 桌面 `16:9`，移动 `9:16`，主视觉 `1:1` 或 `3:2` |
| 迭代 | 先跑 P1 定基调，基调定了再跑其它，别一次全出 |
| 一致性 | 生成时把 P1 的成品当参考图喂给后续提示词 |

**一条最重要的建议**：先只跑 P1，直到那个球"对了"为止。
P1 不对，后面所有场景都是在放大错误。

---

## 1. 设计基调（先看这段，再看提示词）

### 视觉坐标

**不是**：赛博朋克、霓虹紫粉、玻璃拟态、毛玻璃大模糊、赛博 HUD 线框
**是**：深海、安静、单色蓝、体积光、呼吸感、几乎看不见界面

### 三条硬约束

1. **只用一个色相**——全部蓝色系，靠明度/饱和度区分层次。出现第二个色相立刻变廉价。
2. **发光靠叠加**——多层径向渐变叠出核心亮度，模糊只做最后 10%。直接大 blur 会糊成一团。
3. **没有硬边**——球体边缘必须向外衰减到透明，不存在清晰圆周线。

### 固定色板

| 角色 | 色值 | 说明 |
|---|---|---|
| 背景外圈 | `#05070D` | 近黑，带一点蓝 |
| 背景内圈 | `#0B1120` | 径向渐变中心 |
| 核心最亮 | `#E0F2FE` | 接近白 |
| 核心过渡 | `#7DD3FC` | 浅天蓝 |
| 主辉光 | `#22D3EE` | 青色，用来"发光" |
| 深部 | `#2563EB` | 饱和蓝，球体暗面 |
| 粒子 | `#FFFFFF` @ 60% | 纯白半透明 |

### 五个状态（后面提示词会反复用到）

| 态 | 视觉关键词 |
|---|---|
| S0 待机 | 暗、小、慢呼吸、几乎无粒子 |
| S1 唤醒 | 一次亮脉冲 + 一圈光环扩散 |
| S2 聆听 | 粒子缓慢绕流、球体随声起伏 |
| S3 思考 | 粒子加速旋转、偏青、内旋 |
| S4 说话 | 粒子爆发外扩、球体明显脉动 |
| S5 异常 | 转暗、粒子稀疏散落、无辉光 |

---

## 2. 提示词 P1 · 主视觉单帧（先跑这个）

**用途**：定整个项目的视觉基调
**比例**：`1:1`（1024×1024）或 `3:2`
**构图**：球居中，四周大量留黑

### 正向提示词

```
A single luminous abstract sphere floating in absolute darkness, centered
composition with generous negative space around it.

The orb has no hard edge: a brilliant white-cyan core that fades outward
through #7DD3FC into #22D3EE glow, dissolving completely into the black
background. The sphere looks like a dense accumulation of light rather than
a solid object.

Hundreds of tiny white particles orbit the sphere in slow, fluid trajectories,
some close and crisp, some far and blurred, giving a sense of depth and
contained energy.

Background is a very dark navy-black radial gradient, #05070D at the edges
warming to #0B1120 near the orb.

Rendering: cinematic volumetric light, subtle bloom, physically-based
luminescence, ultra clean, minimal, meditative, premium tech product visual.
Color palette strictly monochromatic blue, no other hues.

Shot on a high-end product renderer, 8k, sharp on the core, soft on the
particles, no text, no logo, no UI, no person.
```

### 负向提示词

```
rainbow, purple, pink, magenta, gradient rainbow, cyberpunk, neon signs,
glassmorphism, frosted glass panel, hologram wireframe, HUD elements,
grid lines, text, watermark, logo, hard edges, visible circle outline,
flat vector, 2D illustration, cartoon, sticker, lens flare streak,
lens dirt, chromatic aberration, noisy, grainy, cluttered, busy background,
multiple orbs, planet surface, realistic planet, sun, star, human, face
```

### 参数建议

- Midjourney：`--ar 1:1 --style raw --s 250 --chaos 0`
- Flux：guidance `3.5`，steps `28`
- 出 4–8 张，挑「边缘衰减最自然」的那张

---

## 3. 提示词 P2 · 五态并排对照

**用途**：给开发看状态差异，确认每种态能不能一眼区分
**比例**：`16:9` 横版
**构图**：同一水平线并排 5 个球，从左到右：待机 → 聆听 → 思考 → 说话 → 异常

### 正向提示词

```
Five glowing abstract spheres arranged in a single horizontal row on a pure
black background, evenly spaced, identical in size and position, each
representing a different energy state. Clean comparison chart aesthetic.

Left to right:
1. Dormant — dim, small, almost no glow, a faint blue halo, no particles.
2. Listening — steady medium glow, unhurried slow particles drifting in
   loose orbits, calm and attentive.
3. Thinking — brighter cyan core, particles spinning faster and tighter in
   a clear inward spiral, sense of concentration.
4. Speaking — most energetic, particles bursting outward in all directions,
   the sphere pulsing and radiating visible waves of light.
5. Disconnected — desaturated dark blue, glow almost gone, a few scattered
   particles drifting downward and fading, no energy.

All five share the same design language: no hard edges, soft volumetric
glow, particle systems, monochromatic blue palette from #E0F2FE core through
#22D3EE glow to #2563EB shadows, on #05070D background.

Clean industrial design comparison sheet, generous spacing, no labels,
no text, no numbers, no UI, photorealistic render, 8k.
```

### 负向提示词

```
text, labels, captions, numbers, arrows, UI panels, colored borders,
uneven spacing, different sizes, cluttered, rainbow colors, mixed hues
```

---

## 4. 提示词 P3 · 桌面应用完整界面

**用途**：确定 App 长什么样——球在桌面上的真实观感
**比例**：`16:9`
**构图**：极小无边框窗，球在右下角，桌面可见

### 正向提示词

```
A minimal floating desktop application running on macOS, shown as a very
small borderless transparent window about 240 pixels square, positioned in
the lower right area of a clean dark desktop wallpaper.

Inside the window: a single glowing cyan-blue orb with soft volumetric
particles orbiting it, exactly like a small luminous sphere floating in
darkness. The window itself is invisible — only the orb and its glow are
visible, with no window chrome, no border, no title bar.

Around the orb, at very low opacity, a few ultra-subtle UI hints:
a thin arc showing microphone level, and one minuscule vertical text label
reading "listening". Everything else is the orb.

The desktop wallpaper behind is deep dark blue-grey, soft and out of focus.

Overall feeling: an ambient companion that lives on your desktop, quiet,
non-intrusive, beautiful. Premium Dribbble-quality product shot,
photorealistic, 8k, cinematic soft lighting.
```

### 负向提示词

```
busy desktop, many icons, windows everywhere, taskbar clutter, bright
wallpaper, colorful, glassmorphism, heavy blur, panel, sidebar, dashboard,
multiple widgets, text heavy, cyberpunk UI
```

---

## 5. 提示词 P4 · 移动端 App

**用途**：手机端长什么样
**比例**：`9:16`
**构图**：一部手机正面，屏内只有球 + 极少数文字

### 正向提示词

```
A premium mobile app screen on a modern flagship phone, displayed as a
clean front-facing product mockup on a black background.

The screen is almost entirely dark navy-black #05070D. In the vertical
center sits a glowing cyan-blue orb about one third of the screen width,
with delicate white particles orbiting slowly and a soft volumetric glow
radiating into the darkness.

Above the orb, small and dim: the text "小蝶" in thin light cyan.
Below the orb, a single line of small dim caption: "在".

At the very bottom, a subtle horizontal arc of dots indicating an active
voice session, extremely low contrast.

Nothing else. No navigation bar, no buttons, no cards, no icons.
Extreme minimalism, meditative, premium, ambient.

Photorealistic phone mockup, soft studio lighting, sharp screen details,
8k, product photography.
```

### 负向提示词

```
app icon grid, bottom tab bar, cards, list, chat bubbles, buttons, keyboard,
notifications, colorful, gradient background rainbow, glassmorphism, busy
```

---

## 6. 提示词 P5 · 智能音箱硬件概念

**用途**：将来 ESP32 带小屏音箱长什么样（V3 目标，先出个概念图对齐想象）
**比例**：`1:1` 或 `4:5`
**构图**：产品摄影，一个小型圆润设备摆桌上

### 正向提示词

```
Industrial design concept for a small desktop voice assistant speaker,
product photography on a dark matte surface.

The device is a compact rounded cylinder, roughly the size of a coffee cup,
made of matte dark grey recycled aluminum with a soft fabric mesh lower
section. On its front face is a circular 1.8-inch display.

The display shows only a glowing cyan-blue orb with faint orbiting particles
on near-black background — the same luminous sphere motif, perfectly
circular and centered.

A very subtle ring of light glows around the display bezel, matching the
orb color.

No buttons on the front, no logo, no text. Clean Scandinavian-meets-Japanese
minimal industrial design, like a Teenage Engineering or Muji product.

Studio lighting from upper left, soft shadow, shallow depth of field,
dark moody background, 8k product render, photorealistic.
```

### 负向提示词

```
smart speaker with cloth all around, Amazon Echo, Google Home, HomePod,
fabric sphere, screen with icons, clock display, weather display, buttons,
knobs, LED matrix, RGB rainbow ring, cheap plastic, glossy black
```

---

## 7. 提示词 P6 · 粒子动效分解图

**用途**：给开发看动效的「一拍一拍」怎么走
**比例**：`16:9` 横版，或 `4:5` 竖版分格
**构图**：6–8 格连续帧，表现一次完整交互

### 正向提示词

```
A motion design breakdown sheet showing a single glowing cyan orb across
eight consecutive frames, arranged in a 4x2 grid on a black background.

Frame by frame, the orb evolves:
1. dim sleeping orb, tiny and faint
2. sudden bright flash, a single expanding ring of light
3. ring dissipates, orb settles into gentle steady glow
4. particles begin to waken and lift off the surface
5. particles stream in smooth orbital paths, orb breathing
6. orb compresses inward, particles accelerate and spiral tighter
7. orb expands with a pulse, particles burst outward in all directions
8. everything calms, particles slowly return to lazy drift

Consistent orb design throughout: white-cyan core, #22D3EE volumetric glow,
soft particle system, no hard edges, strictly monochromatic blue.

Technical presentation aesthetic: clean grid, thin dark separators,
no text, no arrows, no numbers, uniform lighting per cell.

Cinematic render, 8k, high detail on particles.
```

### 负向提示词

```
text, labels, arrows, numbers, annotations, film strip borders, different
orb designs per frame, inconsistent color, rainbow, chaotic
```

---

## 8. 提示词 P7 · 集成进 DSH Web GUI

**用途**：作为 DSH 插件时，球怎么和聊天界面共存
**比例**：`16:9`
**构图**：左侧聊天界面，右下角悬浮小球

### 正向提示词

```
A dark-themed developer chat interface, screenshot style, showing an AI
coding agent conversation. Left and center of the frame: a clean minimal
chat panel with monospace code blocks in muted grey-blue tones, dark
background #0B1120, thin subtle dividers.

In the lower right corner, floating above the interface with a soft drop
shadow onto nothing: a small glowing cyan-blue orb, about 100 pixels,
with delicate orbiting white particles and a gentle volumetric halo.

The orb floats freely, not inside any container or card — it appears to
hover over the UI with its own glow bleeding softly onto the dark
background beneath it.

The chat UI itself is understated, almost monochrome, so the orb is clearly
the brightest thing on screen.

Overall: a developer tool with a calm, living presence in the corner.
Photorealistic UI screenshot, sharp text, 8k, minimal.
```

### 负向提示词

```
bright UI, white background, colorful syntax highlighting everywhere,
widgets, sidebar clutter, multiple orbs, glassmorphism card, neon, gradient
cards, rounded card stack, dashboard
```

---

## 9. 提示词 P8 · 品牌 Key Visual

**用途**：项目封面 / 展示图，讲"这是什么"
**比例**：`3:2` 或 `16:9`
**构图**：概念海报，球 + 极少量文字位

### 正向提示词

```
A cinematic brand key visual for a voice AI companion product.

Center-left: a single large luminous cyan-blue orb, dense and alive, with
hundreds of fine white particles streaming around it in elegant arcs,
like a small captured galaxy. Volumetric glow spilling into the darkness.

To the right, negative space of deep navy black, with only three tiny points
of light arranged loosely, suggesting distant tasks being completed.

Between orb and the points of light, extremely faint thin curved lines
suggesting sound waves traveling outward and returning.

Mood: quiet power, focus, intimacy, a companion that listens.
Strictly monochromatic blue palette, no other colors.
Cinematic lighting, ultra-fine particle detail, depth of field,
premium tech brand aesthetic, no text, no logo, 8k, poster quality.
```

### 负向提示词

```
text, logo, watermark, tagline, letters, rainbow, purple, cyberpunk,
circuit board, brain illustration, humanoid robot, face, headphones,
microphone icon, flat illustration
```

---

## 10. 通用负向词库（按需追加）

按你的模型挑，不必全塞：

```
-- 颜色污染
rainbow, purple, magenta, pink, orange, green, multicolor, iridescent

-- 风格跑偏
cyberpunk, vaporwave, synthwave, neon sign, retrowave, HUD, wireframe,
hologram, glitch, pixel art, flat vector, 2D illustration, cartoon,
anime, sticker, emoji, low poly

-- 材质错误
glassmorphism, frosted glass, acrylic, chrome, mirror, liquid metal,
chrome sphere, realistic planet, earth, moon, sun, star, bubble

-- 构图问题
cluttered, busy, dense background, multiple subjects, cropped, cut off,
off-center (除非你要), tilted, asymmetric mess

-- 质量
blurry, low res, jpeg artifacts, noisy, grainy, oversaturated,
overexposed, lens flare streak, lens dirt, chromatic aberration, banding

-- 界面污染
text, letters, words, caption, label, watermark, logo, signature,
UI panel, card, button, icon grid, dashboard, sidebar, tab bar
```

---

## 11. 迭代配方

### 球"糊成一团"

原因：blur 太重 / 核心不亮
改法：加强 `brilliant white-cyan core`，弱化 `bloom`，加 `sharp on the core, soft on the particles`

### 球"太平、没体积"

原因：缺少暗面
改法：加 `subtle shadow on the lower hemisphere, #2563EB on the far side`

### 球"像个星球"

原因：模型把球理解成天体
改法：负向加 `planet, earth, moon, surface texture, terrain, craters, atmosphere rim`，正向强调 `light made solid, not a physical object`

### 粒子"像撒了盐"

原因：粒子无序随机
改法：加 `particles follow smooth orbital trajectories, flowing curves, not random scatter, depth of field on particles`

### 整体"太亮/太闹"

原因：辉光面积过大
改法：加 `90% of the frame is near-black negative space`，负向加 `bright background, glowing everything`

---

## 12. 出图后的验收清单

拿到一张图，按这五条自检：

- [ ] 只有蓝色系？出现第二色相 → 重出
- [ ] 球体边缘是衰减的？有清晰圆周线 → 重出
- [ ] 粒子有前后景深？全在一个平面 → 重出
- [ ] 背景够黑？整体发灰 → 重出
- [ ] 一眼能看出「它在听」？看不出 → 让球更聚光

**过不了就是过不了**，别凑合。UI 的基调一旦歪了，后面所有开发都在给错的东西做实现。

---

## 13. 下一步

1. 跑 P1，出 6–8 张，挑 1 张定稿
2. 拿定稿当参考图，跑 P2 五态，确认状态可区分
3. 跑 P3 / P4，确认桌面与移动观感
4. 把选中的图放进 `Project/蝶梦智能/参考图/`
5. 开发时以这几张为准，别边写边改视觉

> 提示词给了就要一次跑够量。
> 一张一张试永远定不下来。
