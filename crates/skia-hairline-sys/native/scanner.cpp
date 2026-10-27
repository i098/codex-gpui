#include "include/core/SkMatrix.h"
#include "include/core/SkPath.h"
#include "include/core/SkRect.h"
#include "src/core/SkBlitter.h"
#include "src/core/SkMask.h"
#include "src/core/SkPathPriv.h"
#include "src/core/SkRasterClip.h"
#include "src/core/SkScan.h"

#include <cstdint>

namespace {

enum Kind : uint32_t {
    kHorizontal = 0,
    kRectangle = 1,
    kVertical = 2,
    kAntiHorizontal = 3,
    kAntiHorizontalPairOpaque = 4,
    kAntiHorizontalPairAlpha = 5,
    kAntiVerticalPairOpaque = 6,
    kAntiVerticalPairAlpha = 7,
    kMask = 8,
};

using Record = void (*)(void*, uint32_t, int, int, int, int, uint8_t, uint8_t);

class RecordingBlitter final : public SkBlitter {
public:
    RecordingBlitter(uint8_t alpha, Record record, void* context)
        : alpha_(alpha), record_(record), context_(context) {}

    void blitH(int x, int y, int width) override {
        emit(kHorizontal, x, y, width, 1, 255, 0);
    }

    void blitRect(int x, int y, int width, int height) override {
        emit(kRectangle, x, y, width, height, 255, 0);
    }

    void blitV(int x, int y, int height, SkAlpha coverage) override {
        emit(kVertical, x, y, 1, height, coverage, 0);
    }

    void blitAntiH(int x, int y, const SkAlpha coverage[], const int16_t runs[]) override {
        for (int offset = 0; runs[offset] > 0; offset += runs[offset]) {
            emit(kAntiHorizontal, x + offset, y, runs[offset], 1, coverage[offset], 0);
        }
    }

    void blitAntiH2(int x, int y, U8CPU first, U8CPU second) override {
        emit(alpha_ == 255 ? kAntiHorizontalPairOpaque : kAntiHorizontalPairAlpha,
             x, y, 2, 1, first, second);
    }

    void blitAntiV2(int x, int y, U8CPU first, U8CPU second) override {
        emit(alpha_ == 255 ? kAntiVerticalPairOpaque : kAntiVerticalPairAlpha,
             x, y, 1, 2, first, second);
    }

    void blitMask(const SkMask& mask, const SkIRect& clip) override {
        if (mask.fFormat != SkMask::kA8_Format) {
            valid_ = false;
            return;
        }
        for (int y = clip.top(); y < clip.bottom(); ++y) {
            for (int x = clip.left(); x < clip.right(); ++x) {
                emit(kMask, x, y, 1, 1, *mask.getAddr8(x, y), 0);
            }
        }
    }

    bool valid() const { return valid_; }

private:
    void emit(Kind kind, int x, int y, int width, int height, uint8_t first, uint8_t second) {
        record_(context_, kind, x, y, width, height, first, second);
    }

    uint8_t alpha_;
    Record record_;
    void* context_;
    bool valid_ = true;
};

}  // namespace

extern "C" bool skia_hairline_scan(const SkPath* path,
                                    const float* values,
                                    const int* clip_values,
                                    uint8_t alpha,
                                    Record record,
                                    void* context) {
    if (!path || !values || !clip_values || !record ||
        clip_values[0] >= clip_values[2] || clip_values[1] >= clip_values[3]) {
        return false;
    }

    const SkMatrix matrix = SkMatrix::MakeAll(
        values[0], values[1], values[2],
        values[3], values[4], values[5],
        values[6], values[7], values[8]);
    const SkPath device_path = path->makeTransform(matrix);
    auto raw = SkPathPriv::Raw(device_path, SkResolveConvexity::kNo);
    if (!raw) {
        return false;
    }

    const SkRasterClip clip(SkIRect::MakeLTRB(
        clip_values[0], clip_values[1], clip_values[2], clip_values[3]));
    RecordingBlitter recorder(alpha, record, context);
    SkScan::AntiHairPath(*raw, clip, &recorder);
    return recorder.valid();
}
