/*
    SPDX-FileCopyrightText: 2026 the macshell-x contributors

    SPDX-License-Identifier: GPL-3.0-or-later
*/

#include "glasseffect.h"

namespace Macshell
{

KWIN_EFFECT_FACTORY_SUPPORTED(GlassEffect, "metadata.json", return GlassEffect::supported();)

}

#include "main.moc"
