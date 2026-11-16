// Dumps what the pin's stb_truetype (deps_src/imgui/imstb_truetype.h) makes of
#include <iostream>
struct NullLog { template<class T> NullLog &operator<<(const T &) { return *this; } };
#define BOOST_LOG_TRIVIAL(x) NullLog()
// a few glyphs, as Emboss.cpp get_glyph does before healing: flatness, advance,
// each contour's points in shape units (round(p / 0.001)), reversed.
#define STB_TRUETYPE_IMPLEMENTATION
#include "imstb_truetype.h"
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <vector>
int main(int argc, char **argv) {
    FILE *f = fopen(argv[1], "rb"); fseek(f, 0, SEEK_END); long n = ftell(f); rewind(f);
    std::vector<unsigned char> d(n); fread(d.data(), 1, n, f); fclose(f);
    stbtt_fontinfo info; stbtt_InitFont(&info, d.data(), stbtt_GetFontOffsetForIndex(d.data(), 0));
    int ascent, descent, gap; stbtt_GetFontVMetrics(&info, &ascent, &descent, &gap);
    float size = argc > 3 ? (float)atof(argv[3]) : 4.f;
    const float RESOLUTION = 0.0125f;
    float flatness = ascent * RESOLUTION / size; if (flatness < RESOLUTION) flatness = RESOLUTION;
    printf("ascent %d descent %d gap %d flatness %.9g\n", ascent, descent, gap, flatness);
    for (const char *c = argv[2]; *c; ++c) {
        int g = stbtt_FindGlyphIndex(&info, *c);
        int adv, lsb; stbtt_GetGlyphHMetrics(&info, g, &adv, &lsb);
        stbtt_vertex *v; int nv = stbtt_GetGlyphShape(&info, g, &v);
        int *lens = 0, nc = 0;
        stbtt__point *pts = nv > 0 ? stbtt_FlattenCurves(v, nv, flatness, &lens, &nc, 0) : 0;
        printf("glyph %c index %d advance %d contours %d\n", *c, g, adv, nc);
        int pi = 0;
        for (int ci = 0; ci < nc; ++ci) {
            int len = lens[ci];
            printf(" contour %d\n", len);
            for (int i = 0; i < len; ++i, ++pi)
                printf("  %ld %ld\n", (long)std::round(pts[pi].x / 0.001), (long)std::round(pts[pi].y / 0.001));
        }
    }
}
