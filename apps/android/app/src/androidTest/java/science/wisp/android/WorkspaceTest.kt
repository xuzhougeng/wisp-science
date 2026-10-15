package science.wisp.android

import android.view.KeyEvent
import android.graphics.Bitmap
import java.io.File
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.text.TextLayoutResult
import java.time.LocalDate
import java.time.YearMonth
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.test.core.app.ActivityScenario
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test

class WorkspaceTest {
    @get:Rule val compose = createEmptyComposeRule()
    private lateinit var activity: ActivityScenario<MainActivity>

    @Before fun startOffline() {
        // Set before Activity creation: tests never need or initiate a remote connection.
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        context.getSharedPreferences("workspace", 0).edit().putBoolean("demo", true).remove("transcript_detail").commit()
        activity = ActivityScenario.launch(MainActivity::class.java)
    }
    @After fun close() { activity.close() }

    // Opt-in visual artifacts for emulator review; no real host data is used.
    private fun capture(name: String) {
        if (InstrumentationRegistry.getArguments().getString("screenshots") != "true") return
        compose.waitForIdle()
        val instrumentation = InstrumentationRegistry.getInstrumentation()
        val directory = File(instrumentation.targetContext.getExternalFilesDir(null), "screenshots").apply { mkdirs() }
        val bitmap = requireNotNull(instrumentation.uiAutomation.takeScreenshot())
        File(directory, "$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
        bitmap.recycle()
    }

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
        compose.onNodeWithContentDescription("消息").assertExists()
        escape()
        compose.onNodeWithText("分析流程与审批").assertExists()
        escape()
        compose.onNodeWithText("记录本").assertExists()
    }

    @Test fun transcriptDetailMenuClosesFirstOnEscapeAndPersistsTheChoice() {
        compose.onNodeWithText("RNA-seq 质控 · 演示").performClick()
        compose.onNodeWithText("样本质控与报告").performClick()
        compose.onNodeWithContentDescription("信息详细程度").performClick()
        compose.onNodeWithText("标准").assertIsSelected()
        escape()
        compose.onNodeWithTag("transcript-detail-menu").assertDoesNotExist()
        compose.onNodeWithContentDescription("消息").assertExists()
        compose.onNodeWithContentDescription("信息详细程度").performClick()
        compose.onNodeWithText("简洁").performClick()
        compose.onNodeWithTag("transcript-detail-menu").assertDoesNotExist()
        val preferences = InstrumentationRegistry.getInstrumentation().targetContext.getSharedPreferences("workspace", 0)
        assertEquals("minimal", preferences.getString("transcript_detail", null))
        activity.recreate()
        compose.onNodeWithContentDescription("信息详细程度").performClick()
        compose.onNodeWithText("简洁").assertIsSelected()
        compose.onNodeWithText("标准").assertIsNotSelected()
        escape()
        compose.onNodeWithContentDescription("消息").assertExists()
        escape()
        compose.onNodeWithText("分析流程与审批").assertExists()
    }

    @Test fun demoMessageHasLocalReplyAndSurvivesActivityRecreation() {
        compose.onNodeWithText("RNA-seq 质控 · 演示").performClick()
        compose.onNodeWithText("样本质控与报告").performClick()
        compose.onNodeWithContentDescription("消息").performTextInput("检查示例")
        activity.recreate()
        compose.onNodeWithText("检查示例").assertExists()
        compose.onNodeWithText("发送").assertIsEnabled().performClick()
        compose.onNodeWithTag("transcript").performScrollToNode(hasText("本地预设回复", substring = true))
        compose.onNodeWithText("本地预设回复", substring = true).assertIsDisplayed()
    }

    @Test fun offlineNotebookSavesTextAndSurvivesRecreation() {
        compose.onNodeWithText("记录本").performClick()
        compose.onNodeWithText("记下刚才的观察").performClick()
        val note = "Android 离线观察 ${System.nanoTime()}"
        compose.onNodeWithText("记录内容").performTextInput(note)
        compose.onNodeWithText("保存到本机").performClick()
        compose.waitUntil(5_000) { compose.onAllNodesWithText("保存到本机").fetchSemanticsNodes().isEmpty() }
        activity.recreate()
        compose.onAllNodesWithText(note).onFirst().assertExists()
        escape()
        compose.onNodeWithText("清单").assertExists()
    }

    @Test fun settingsEscapeDoesNotLeaveTheProjectList() {
        compose.onNodeWithContentDescription("连接设置").performClick()
        escape()
        compose.onNodeWithText("RNA-seq 质控 · 演示").assertExists()
    }
    @Test fun tabsPreserveProjectNavigationAndCalendarSelection() {
        compose.onNodeWithText("RNA-seq 质控 · 演示").assertExists()
        capture("projects")
        compose.onNodeWithText("RNA-seq 质控 · 演示").performClick()
        capture("sessions")
        compose.onNodeWithText("日历").performClick()
        compose.onNodeWithTag("calendar-filter-demo-rnaseq").performClick().assertIsSelected()
        compose.onNodeWithContentDescription("上个月").performClick()
        val previous = YearMonth.now().minusMonths(1)
        compose.onNodeWithTag("calendar-month").assertTextEquals("${previous.year}年${previous.monthValue}月")
        compose.onNodeWithText("记录本").performClick()
        compose.onNodeWithText("日历").performClick()
        compose.onNodeWithTag("calendar-month").assertTextEquals("${previous.year}年${previous.monthValue}月")
        compose.onNodeWithText("今天").performClick()
        compose.onNodeWithTag("calendar-day-${LocalDate.now()}").assertIsSelected()
        capture("calendar")
        compose.onNodeWithText("打开关联对话").performScrollTo().performClick()
        compose.onNodeWithContentDescription("项目文件").assertExists()
        escape()
        compose.onNodeWithText("科研日历").assertExists()
        compose.onNodeWithText("对话").performClick()
        compose.onNodeWithText("分析流程与审批").assertExists()
        escape()
        compose.onNodeWithContentDescription("连接设置").assertExists()
    }

    @Test fun checklistCanBeRescheduledCompletedAndRestored() {
        val task = "日期回归 ${System.nanoTime()}"
        compose.onNodeWithText("清单").performClick()
        compose.onNodeWithTag("checklist-input").performTextInput(task)
        compose.onNodeWithText("添加").performClick()
        compose.waitUntil(5_000) { compose.onAllNodesWithText(task).fetchSemanticsNodes().size == 1 }
        compose.onNodeWithText(task).performClick()
        compose.onNodeWithText("日期（YYYY-MM-DD，可留空）").performTextReplacement("2026-02-30")
        compose.onNodeWithText("保存到本机").assertIsNotEnabled()
        compose.onNodeWithText("日期（YYYY-MM-DD，可留空）").performTextReplacement(LocalDate.now().minusDays(1).toString())
        compose.onNodeWithText("保存到本机").performClick()
        compose.waitUntil(5_000) { compose.onAllNodesWithText("保存到本机").fetchSemanticsNodes().isEmpty() }
        compose.onNodeWithText("逾期", substring = true).assertExists()
        compose.onNodeWithContentDescription("完成：$task").performClick()
        compose.onNodeWithText("已完成 ·", substring = true).performClick()
        compose.onNodeWithContentDescription("恢复待办：$task").performScrollTo().performClick()
        activity.recreate()
        compose.onNodeWithContentDescription("完成：$task").assertExists()
        capture("checklist")
    }

    @Test fun notebookExampleAndHelpReturnOneLayerAndNewEditorStartsEmpty() {
        compose.onNodeWithText("记录本").performClick()
        compose.onNodeWithText("拍下纸质实验记录").assertIsNotEnabled()
        capture("notebook")
        compose.onNodeWithText("先看一条示例").performClick()
        escape()
        compose.onNodeWithText("你现在想做什么？").assertExists()
        compose.onNodeWithContentDescription("记录本帮助").performClick()
        escape()
        compose.onNodeWithText("记下刚才的观察").performClick()
        compose.onNodeWithText("记录内容").performTextInput("未保存的观察")
        escape()
        compose.onNodeWithText("记下刚才的观察").performClick()
        compose.onNodeWithText("未保存的观察").assertDoesNotExist()
    }

    @Test fun successfulRunsAreCollapsedAndFailureRemainsVisible() {
        compose.onNodeWithText("RNA-seq 质控 · 演示").performClick()
        compose.onNodeWithText("样本质控与报告").performClick()
        compose.onNodeWithTag("transcript").performScrollToNode(hasText("已完成的记录（2）"))
        compose.onNodeWithText("检查样本表与分组").assertDoesNotExist()
        compose.onNodeWithText("已完成的记录（2）").performClick()
        compose.onNodeWithTag("transcript").performScrollToNode(hasText("检查样本表与分组"))
        compose.onNodeWithText("检查样本表与分组").performClick()
        compose.onNodeWithText("[DEMO] Loaded", substring = true).assertExists()
        escape()
        compose.onNodeWithText("分析流程与审批").performClick()
        compose.onNodeWithTag("transcript").performScrollToNode(hasText("读取示例输入文件（失败记录演示）"))
        compose.onNodeWithText("失败").assertExists()
        capture("conversation")
    }

    @Test fun inboxOpensPendingConversationAndBackKeepsInbox() {
        compose.onNodeWithContentDescription("待处理对话").performClick()
        compose.onNodeWithText("分析流程与审批").performClick()
        escape()
        compose.onNodeWithText("待处理对话").assertExists()
        escape()
        compose.onNodeWithText("RNA-seq 质控 · 演示").assertExists()
    }
    @Test fun markdownRendersTableImageAndLinksToFilePreviews() {
        compose.onNodeWithText("RNA-seq 质控 · 演示").performClick()
        compose.onNodeWithText("样本质控与报告").performClick()
        compose.onNodeWithText("示例质控已整理").performScrollTo().assertIsDisplayed()
        compose.onNodeWithText("# 示例质控已整理", substring = true).assertDoesNotExist()
        compose.onNodeWithText("Control_01").performScrollTo().assertIsDisplayed()
        compose.onNodeWithTag("markdown-table").assertExists()
        capture("markdown-table")
        compose.onNodeWithTag("markdown-table").performTouchInput { swipeLeft() }
        compose.onNodeWithText("Q30").assertIsDisplayed()
        compose.onNodeWithText("93.8%").assertIsDisplayed()
        compose.onNodeWithTag("markdown-table").performTouchInput { swipeRight() }
        compose.waitUntil(10_000) { compose.onAllNodesWithTag("markdown-image").fetchSemanticsNodes().isNotEmpty() }
        compose.onNodeWithTag("markdown-image").performScrollTo().assertIsDisplayed()
        capture("markdown-image")
        val report = compose.onNodeWithText("打开完整报告", substring = true).performScrollTo()
        val layouts = mutableListOf<TextLayoutResult>()
        report.performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(layouts) }
        val layout = layouts.single()
        val index = layout.layoutInput.text.text.indexOf("打开完整报告")
        report.performTouchInput { click(layout.getBoundingBox(index).center) }
        compose.onNodeWithText("qc_report.md").assertExists()
        compose.onNodeWithText("RNA-seq 质控报告", substring = true).assertExists()
        capture("markdown-file")
        val metrics = compose.onNodeWithText("查看原始示例指标", substring = true).performScrollTo()
        val metricsLayouts = mutableListOf<TextLayoutResult>()
        metrics.performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(metricsLayouts) }
        metrics.performTouchInput { click(metricsLayouts.single().getBoundingBox(0).center) }
        compose.onNodeWithText("qc_metrics.csv").assertExists()
        compose.onNodeWithText("Control_01", substring = true).assertExists()
        escape()
        compose.onNodeWithText("qc_report.md").assertExists()
        escape()
        compose.onNodeWithContentDescription("项目文件").assertExists()
    }
}
