# wingbar — Brand Colour Palette

Derived from the app icon ("Titanium Black", pearl wings, cool specular rim) and the native macOS utility interaction model.

---

### 1. Palette Specification

| Token Name | Hex Code | RGB | Primary Role & Semantic Usage |
| :--- | :---: | :---: | :--- |
| **Titanium 950** (Midnight Void) | `#0A0B0D` | `(10, 11, 13)` | Dark canvas background, deepest surface, squircle base bottom, menu notch. |
| **Titanium 900** (Deep Slate) | `#14161A` | `(20, 22, 26)` | Dark panel/card background, widget background, dropdown menu surface. |
| **Titanium 800** (Charcoal Horizon) | `#282C34` | `(40, 44, 52)` | Squircle base top, elevated widget fill, light-theme primary text & glyph fill. |
| **Titanium 600** (Muted Steel) | `#5A6072` | `(90, 96, 114)` | Light-theme muted text & borders, subtle dividers. |
| **Titanium 500** (Cool Slate) | `#727A91` | `(114, 122, 145)` | Dark-theme muted text, subtle telemetry labels (WCAG 4.6:1 AA on `#0A0B0D`). |
| **Pearl 200** (Cool Mist) | `#D5DCE6` | `(213, 220, 230)` | Dark-theme glyph bottom stop, light-theme secondary surface & card borders. |
| **Pearl 100** (Pearl White) | `#F2F5F9` | `(242, 245, 249)` | Dark-theme glyph mid stop, light-theme elevated surface fill. |
| **Pearl 50** (Pure Specular) | `#FFFFFF` | `(255, 255, 255)` | Dark-theme text primary & glyph specular glint, light-theme canvas background. |
| **Specular Ice** (Rim Blue) | `#94B7E8` | `(148, 183, 232)` | Icon specular rim stroke, active keyboard focus ring, cool accent. |
| **Wing Green** (Status Active) | `#9ED072` | `(158, 208, 114)` | Dark-theme brand accent, active workspace/CI passing indicator, online status. |
| **Emerald Deep** (Light Mode Accent) | `#2D6B12` | `(45, 107, 18)` | Light-theme accessible brand accent, interactive links, high-contrast badges. |
| **Terminal Cyan** (Status Info) | `#76CCE0` | `(118, 204, 224)` | Dark-theme secondary telemetry accent (CPU/RAM/network status). |
| **Cyan Deep** (Light Mode Info) | `#12687A` | `(18, 104, 122)` | Light-theme secondary telemetry text and high-contrast info tags. |

---

### 2. Accent Family Justification

wingbar is an unobtrusive, precision macOS status bar designed for power users and tiling window managers (Yabai, AeroSpace). It draws live telemetry and widgets around the screen notch.
- **Why Wing Green (`#9ED072`)**: Green is the native signal of operational health, active workspaces, and passing CI builds (`CI ✓`). `#9ED072` is a calm, desaturated spring green that feels organic rather than synthetic, matching macOS's traffic light green and Activity Monitor meters.
- **Why Terminal Cyan (`#76CCE0`)**: A cool, calm secondary telemetry accent that harmonizes with the `#94B7E8` specular rim and reflects the developer terminal heritage (Ghostty, Alacritty) without competing with the primary green.

---

### 3. Light and Dark Theme Mappings

| Semantic Role | Dark Theme Token | Light Theme Token |
| :--- | :--- | :--- |
| **Canvas Background** | Titanium 950 (`#0A0B0D`) | Pearl 50 (`#FFFFFF`) |
| **Surface / Card Background** | Titanium 900 (`#14161A`) | Pearl 100 (`#F2F5F9`) |
| **Elevated Surface / Popover** | Titanium 800 (`#282C34`) | Pearl 50 (`#FFFFFF`) |
| **Borders & Dividers** | Titanium 800 (`#282C34`) | Pearl 200 (`#D5DCE6`) |
| **Primary Text / Glyph** | Pearl 50 (`#FFFFFF`) | Titanium 950 (`#0A0B0D`) |
| **Secondary Text** | Pearl 200 (`#D5DCE6`) | Titanium 800 (`#282C34`) |
| **Muted Text** | **Titanium 500 (`#727A91`)** | **Titanium 600 (`#5A6072`)** |
| **Primary Accent (Status)** | Wing Green (`#9ED072`) | Emerald Deep (`#2D6B12`) |
| **Secondary Accent (Info)** | Terminal Cyan (`#76CCE0`) | Cyan Deep (`#12687A`) |
| **Focus Ring / Specular** | Specular Ice (`#94B7E8`) | Specular Ice (`#94B7E8`) |

---

### 4. WCAG Contrast Compliance (Text on Background)

All recommended pairs comfortably exceed the WCAG AA threshold ($\ge 4.5:1$ for normal text):

| Pair Usage | Foreground | Background | Contrast Ratio | WCAG Compliance |
| :--- | :--- | :--- | :---: | :---: |
| **Dark: Primary Text** | Pearl 50 (`#FFFFFF`) | Titanium 950 (`#0A0B0D`) | **19.69:1** | **AAA** |
| **Dark: Secondary Text** | Pearl 200 (`#D5DCE6`) | Titanium 950 (`#0A0B0D`) | **14.26:1** | **AAA** |
| **Dark: Muted Text** | **Titanium 500 (`#727A91`)** | **Titanium 950 (`#0A0B0D`)** | **4.60:1** | **AA** |
| **Dark: Active Accent** | Wing Green (`#9ED072`) | Titanium 950 (`#0A0B0D`) | **10.99:1** | **AAA** |
| **Dark: Info Accent** | Terminal Cyan (`#76CCE0`) | Titanium 950 (`#0A0B0D`) | **10.77:1** | **AAA** |
| **Light: Primary Text** | Titanium 950 (`#0A0B0D`) | Pearl 50 (`#FFFFFF`) | **19.69:1** | **AAA** |
| **Light: Secondary Text** | Titanium 800 (`#282C34`) | Pearl 50 (`#FFFFFF`) | **14.00:1** | **AAA** |
| **Light: Secondary Text on Surface** | Titanium 800 (`#282C34`) | Pearl 100 (`#F2F5F9`) | **12.80:1** | **AAA** |
| **Light: Muted Text** | Titanium 600 (`#5A6072`) | Pearl 50 (`#FFFFFF`) | **6.27:1** | **AA** |
| **Light: Active Accent** | Emerald Deep (`#2D6B12`) | Pearl 50 (`#FFFFFF`) | **6.51:1** | **AA** |
| **Light: Info Accent** | Cyan Deep (`#12687A`) | Pearl 50 (`#FFFFFF`) | **6.39:1** | **AA** |
