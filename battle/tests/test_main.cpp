// battle_tests 入口：挂载内容后依次运行各组无网络测试。

#include "battle/BattleContent.h"
#include "test_support.h"

#include <cstdio>
#include <string>

void runBattleRoomTests();
void runNetDuelTests();

int main(int argc, char* argv[])
{
    std::string contentRoot = MUGEN_CONTENT_DIR;
    if (argc > 1)
        contentRoot = argv[1];

    if (!battle::mountBattleContent(contentRoot))
    {
        std::fprintf(stderr, "failed to mount content at %s\n", contentRoot.c_str());
        return 1;
    }

    runBattleRoomTests();
    runNetDuelTests();

    if (battle_test::g_failures != 0)
    {
        std::fprintf(stderr, "%d test(s) failed\n", battle_test::g_failures);
        return 1;
    }
    std::printf("all battle tests passed\n");
    return 0;
}
