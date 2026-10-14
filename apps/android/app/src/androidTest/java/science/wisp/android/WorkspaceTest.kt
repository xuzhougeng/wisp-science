package science.wisp.android

import android.view.KeyEvent
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class WorkspaceTest {
    @get:Rule val compose = createEmptyComposeRule()
    private lateinit var activity: ActivityScenario<MainActivity>

    @Before fun startOffline() {
        // Set before Activity creation: tests never need or initiate a remote connection.
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        context.getSharedPreferences("workspace", 0).edit().putBoolean("demo", true).commit()
        activity = ActivityScenario.launch(MainActivity::class.java)
    }
    @After fun close() { activity.close() }

    private fun escape() {
        // Send to the window immediately, without moving focus into the new page.
        activity.onActivity {
            it.window.callback.dispatchKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, KeyEvent.KEYCODE_ESCAPE))
            it.window.callback.dispatchKeyEvent(KeyEvent(KeyEvent.ACTION_UP, KeyEvent.KEYCODE_ESCAPE))
        }
    }

    @Test fun demoNavigationAndEscapeCloseExactlyOneLevel() {
        compose.onNodeWithText("RNA-seq 质控 · 演示").performClick()
        compose.onNodeWithText("样本质控与报告").performClick()
        compose.onNodeWithContentDescription("项目文件").performClick()
        escape()
        compose.onNodeWithContentDescription("项目文件").assertExists()
        compose.onNodeWithText("消息").assertExists()
        escape()
        compose.onNodeWithText("分析流程与审批").assertExists()
        escape()
        compose.onNodeWithText("实验记录本").assertExists()
    }

    @Test fun demoMessageHasLocalReplyAndSurvivesActivityRecreation() {
        compose.onNodeWithText("RNA-seq 质控 · 演示").performClick()
        compose.onNodeWithText("样本质控与报告").performClick()
        compose.onNodeWithText("消息").performTextInput("检查示例")
        activity.recreate()
        compose.onNodeWithText("检查示例").assertExists()
        compose.onNodeWithText("发送").assertIsEnabled().performClick()
        compose.onNodeWithTag("transcript").performScrollToNode(hasText("本地预设回复", substring = true))
        compose.onNodeWithText("本地预设回复", substring = true).assertIsDisplayed()
    }

    @Test fun offlineNotebookSavesTextAndSurvivesRecreation() {
        compose.onNodeWithText("实验记录本").performClick()
        compose.onNodeWithText("新增记录").performClick()
        val note = "Android 离线观察 ${System.nanoTime()}"
        compose.onNodeWithText("记录内容").performTextInput(note)
        compose.onNodeWithText("保存到本机").performClick()
        compose.waitUntil(5_000) { compose.onAllNodesWithText("保存到本机").fetchSemanticsNodes().isEmpty() }
        activity.recreate()
        compose.onAllNodesWithText(note).onFirst().assertExists()
        escape()
        compose.onNodeWithText("科研清单").assertExists()
    }

    @Test fun settingsEscapeDoesNotLeaveTheProjectList() {
        compose.onNodeWithContentDescription("连接设置").performClick()
        escape()
        compose.onNodeWithText("RNA-seq 质控 · 演示").assertExists()
    }
}
