#!/bin/sh
# gputest.sh: what renders Arcan, the GPU's bring-up, and an OpenGL test
# on the GPU (OSMesa on freedreno through /dev/dri/card0).
cat /tmp/arcan/renderer 2>/dev/null
sysctl -n hw.mi5.gpu | grep -E 'a5xx: (smmu refuses|CP_|out of|start fin)'
OSMESA_DRM_DEVICE=/dev/dri/card0 gltest 2>&1 | tail -6
