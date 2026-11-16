//! The icons of the "wings" theme of the simple-bar setup wingbar replaced
//! (its own line-art SVGs, drawn where simple-bar has no icon or a different
//! one: memory, storage, Ethernet, the volume levels, each prayer). Whole SVG
//! documents drawn in `black`, which `svg` replaces with the tint; sorted by
//! name, and no name is also one of simple-bar's.

/// (name, SVG document), sorted by name.
pub static ICONS: &[(&str, &str)] = &[
    (
        "Asr",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"black\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M2 20.5h20\"/><path d=\"M10.5 20.5V11\"/><path d=\"M10.5 20.5H3\" stroke-width=\"3.4\"/><circle cx=\"17.5\" cy=\"7\" r=\"2.6\"/><path d=\"M17.5 2.2v1M21.8 7h-1M20.6 3.9l-.7.7\"/></svg>",
    ),
    (
        "Chorouq",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"black\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M2 19h20\"/><path d=\"M6.5 19a5.5 5.5 0 0 1 11 0\"/><path d=\"M12 6v3M4.6 9.6l2 2M19.4 9.6l-2 2\"/></svg>",
    ),
    (
        "Dohr",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"black\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><circle cx=\"12\" cy=\"12\" r=\"4.2\"/><path d=\"M12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M5.3 18.7l1.4-1.4M17.3 6.7l1.4-1.4\"/></svg>",
    ),
    (
        "Ethernet",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"black\" stroke-width=\"2.2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><rect x=\"3\" y=\"4\" width=\"18\" height=\"13\" rx=\"2\"/><path d=\"M8 17v3h8v-3M7 8v3M10 8v3M14 8v3M17 8v3\"/></svg>",
    ),
    (
        "Fajr",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"black\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M2 19h20\"/><path d=\"M12 16v-4M7.5 16.5 5.5 14M16.5 16.5l2-2.5\"/><path d=\"M18 3.5v3M16.5 5h3\"/><path d=\"M6 6.5v1.5M5.25 7.25h1.5\" stroke-width=\"1.6\"/></svg>",
    ),
    (
        "Ishae",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"black\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M19.5 14.5A8 8 0 0 1 9.5 4.5a8 8 0 1 0 10 10z\"/><path d=\"M17 3.5v3M15.5 5h3\" stroke-width=\"1.6\"/><path d=\"M21 9.5v2M20 10.5h2\" stroke-width=\"1.4\"/></svg>",
    ),
    (
        "Maghrib",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"black\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M2 19h20\"/><path d=\"M6.5 19a5.5 5.5 0 0 1 11 0\"/><path d=\"M12 3.5v5.5M9.5 6.5 12 9l2.5-2.5\"/></svg>",
    ),
    (
        "Memory",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"black\" stroke-width=\"1.8\" stroke-linejoin=\"round\"><path d=\"M3 6h18a1 1 0 0 1 1 1v9h-8.5v-1.5h-3V16H2V7a1 1 0 0 1 1-1z\"/><g fill=\"black\" stroke=\"none\"><rect x=\"4.5\" y=\"8.5\" width=\"4\" height=\"4.5\" rx=\".5\"/><rect x=\"10\" y=\"8.5\" width=\"4\" height=\"4.5\" rx=\".5\"/><rect x=\"15.5\" y=\"8.5\" width=\"4\" height=\"4.5\" rx=\".5\"/></g><path d=\"M4.5 16v3M7.5 16v3M16.5 16v3M19.5 16v3\" stroke-linecap=\"round\"/></svg>",
    ),
    (
        "Speaker1",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"black\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M3 9.5v5a1 1 0 0 0 1 1h3l4.4 3.6a.6.6 0 0 0 1-.5V5.4a.6.6 0 0 0-1-.5L7 8.5H4a1 1 0 0 0-1 1z\" fill=\"black\"/><path d=\"M15.5 9.5a3.5 3.5 0 0 1 0 5\"/></svg>",
    ),
    (
        "Speaker2",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"black\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M3 9.5v5a1 1 0 0 0 1 1h3l4.4 3.6a.6.6 0 0 0 1-.5V5.4a.6.6 0 0 0-1-.5L7 8.5H4a1 1 0 0 0-1 1z\" fill=\"black\"/><path d=\"M15.5 9.5a3.5 3.5 0 0 1 0 5\"/><path d=\"M18 7a7 7 0 0 1 0 10\"/></svg>",
    ),
    (
        "Speaker3",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"black\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M3 9.5v5a1 1 0 0 0 1 1h3l4.4 3.6a.6.6 0 0 0 1-.5V5.4a.6.6 0 0 0-1-.5L7 8.5H4a1 1 0 0 0-1 1z\" fill=\"black\"/><path d=\"M15.5 9.5a3.5 3.5 0 0 1 0 5\"/><path d=\"M18 7a7 7 0 0 1 0 10\"/><path d=\"M20.5 4.5a10.5 10.5 0 0 1 0 15\"/></svg>",
    ),
    (
        "SpeakerMuted",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"black\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M3 9.5v5a1 1 0 0 0 1 1h3l4.4 3.6a.6.6 0 0 0 1-.5V5.4a.6.6 0 0 0-1-.5L7 8.5H4a1 1 0 0 0-1 1z\" fill=\"black\"/><path d=\"M16 9.5l5 5M21 9.5l-5 5\"/></svg>",
    ),
    (
        "Storage",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"black\" stroke-width=\"2.2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M22 12H2\"/><path d=\"M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z\"/><path d=\"M6 16h.01M10 16h.01\"/></svg>",
    ),
];
