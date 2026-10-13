import XCTest

final class WorkspaceUITests: XCTestCase {
    private func launch(_ mode: String = "connected", darkAndLarge: Bool = false) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchEnvironment["WISP_UI_TESTING"] = mode
        if darkAndLarge {
            app.launchArguments += ["-AppleInterfaceStyle", "Dark", "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"]
        }
        app.launch()
        return app
    }
    func testCachedProjectConversationAndFileSurviveOfflineRelaunchAndClear() {
        let id = UUID().uuidString
        func start(_ mode: String) -> XCUIApplication {
            let app = XCUIApplication()
            app.launchEnvironment["WISP_UI_TESTING"] = mode
            app.launchEnvironment["WISP_CACHE_TEST_ID"] = id
            app.launch()
            return app
        }
        let seed = start("cache-seed")
        openConversation(seed)
        seed.buttons["conversation-files"].tap()
        XCTAssertTrue(seed.staticTexts["report.md"].waitForExistence(timeout: 5))
        seed.staticTexts["report.md"].tap()
        XCTAssertTrue(seed.staticTexts["质控通过。"].waitForExistence(timeout: 5))
        seed.terminate()
        let offline = start("cache-offline")
        openConversation(offline)
        XCTAssertTrue(offline.staticTexts["cached-conversation"].waitForExistence(timeout: 5))
        XCTAssertFalse(offline.buttons["send-message"].isEnabled)
        offline.buttons["conversation-files"].tap()
        XCTAssertTrue(offline.staticTexts["report.md"].waitForExistence(timeout: 5))
        offline.staticTexts["report.md"].tap()
        XCTAssertTrue(offline.staticTexts["质控通过。"].waitForExistence(timeout: 5))
        XCTAssertTrue(offline.staticTexts["cached-file"].exists)
        offline.typeKey(XCUIKeyboardKey.escape, modifierFlags: [])
        for _ in 0..<2 {
            if offline.buttons["connection-settings"].isHittable { break }
            offline.navigationBars.buttons.firstMatch.tap()
        }
        offline.buttons["connection-settings"].tap()
        let clear = offline.buttons["clear-cache"]
        for _ in 0..<4 { if clear.isHittable { break }; offline.swipeUp() }
        clear.tap()
        XCTAssertTrue(offline.staticTexts["本机缓存已清除，后续读取时会重新缓存。"].waitForExistence(timeout: 5))
        offline.terminate()
        let empty = start("cache-offline")
        XCTAssertTrue(empty.staticTexts["连接后显示项目。"].waitForExistence(timeout: 5))
        XCTAssertFalse(empty.staticTexts["RNA-seq 研究"].exists)
    }
    private func openConversation(_ app: XCUIApplication, title: String = "样本质控") {
        XCTAssertTrue(app.staticTexts["RNA-seq 研究"].firstMatch.waitForExistence(timeout: 5))
        app.staticTexts["RNA-seq 研究"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts[title].firstMatch.waitForExistence(timeout: 5))
        app.staticTexts[title].firstMatch.tap()
        XCTAssertTrue(app.buttons["conversation-files"].waitForExistence(timeout: 5))
    }

    func testWorkspaceListTitlesShareTheToolbarRow() {
        verifyWorkspaceNavigation(darkAndLarge: false)
    }

    func testWorkspaceListTitlesShareTheToolbarRowWithAccessibilityText() {
        verifyWorkspaceNavigation(darkAndLarge: true)
    }

    private func verifyWorkspaceNavigation(darkAndLarge: Bool) {
        let app = launch(darkAndLarge: darkAndLarge)
        let project = app.staticTexts["RNA-seq 研究"].firstMatch
        XCTAssertTrue(project.waitForExistence(timeout: 5))
        assertTitleSharesToolbarRow(app, title: "Wisp", action: "connection-settings")
        assertTitleSharesToolbarRow(app, title: "Wisp", action: "inbox")
        app.buttons["connection-settings"].tap()
        XCTAssertTrue(app.buttons["完成"].waitForExistence(timeout: 5))
        app.typeKey(XCUIKeyboardKey.escape, modifierFlags: [])
        project.tap()
        XCTAssertTrue(app.staticTexts["样本质控"].firstMatch.waitForExistence(timeout: 5))
        assertTitleSharesToolbarRow(app, title: "RNA-seq 研究", action: "new-conversation")
    }

    private func assertTitleSharesToolbarRow(_ app: XCUIApplication, title: String, action: String,
                                             file: StaticString = #filePath, line: UInt = #line) {
        let heading = app.navigationBars.staticTexts[title].firstMatch
        let button = app.buttons[action]
        // A large title exists in accessibility too, but sits below the controls
        // and consumes another row before the first project or conversation.
        let sameRow = NSPredicate { _, _ in
            guard heading.exists, button.exists, !heading.frame.isEmpty, !button.frame.isEmpty else { return false }
            return heading.frame.midY >= button.frame.minY && heading.frame.midY <= button.frame.maxY
        }
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: sameRow, object: app)], timeout: 5),
                       .completed, file: file, line: line)
        XCTAssertTrue(button.isHittable, file: file, line: line)
        XCTAssertFalse(heading.frame.intersects(button.frame), file: file, line: line)
    }

    func testPhoneAndPadNavigateReadFilesAndDismissWithEscapeImmediately() {
        let app = launch()
        openConversation(app)
        app.buttons["conversation-files"].tap()
        XCTAssertTrue(app.buttons["完成"].waitForExistence(timeout: 5))
        // No focus movement inside the sheet before Escape.
        app.typeKey(XCUIKeyboardKey.escape, modifierFlags: [])
        XCTAssertTrue(app.buttons["conversation-files"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["完成"].exists)
        app.buttons["conversation-files"].tap()
        XCTAssertTrue(app.staticTexts["report.md"].waitForExistence(timeout: 5))
        app.staticTexts["report.md"].tap()
        XCTAssertTrue(app.staticTexts["质控通过。"].waitForExistence(timeout: 5))
        app.typeKey(XCUIKeyboardKey.escape, modifierFlags: [])
        XCTAssertTrue(app.buttons["send-message"].exists)
    }
    func testSuggestionFillsDraftAndSendWaitsForExplicitTap() {
        let app = launch()
        openConversation(app)
        XCTAssertTrue(app.buttons["查看统计结果"].waitForExistence(timeout: 5))
        app.buttons["查看统计结果"].tap()
        let draft = app.textFields["message-draft"].exists ? app.textFields["message-draft"] : app.textViews["message-draft"]
        XCTAssertEqual(draft.value as? String, "查看统计结果")
        XCTAssertFalse(app.staticTexts["已收到离线测试消息。"].exists)
        app.buttons["send-message"].tap()
        XCTAssertTrue(app.staticTexts["已收到离线测试消息。"].waitForExistence(timeout: 5))
    }
    func testApprovalAndStopUseRemoteCapabilities() {
        let app = launch("running")
        openConversation(app)
        XCTAssertTrue(app.buttons["批准一次"].waitForExistence(timeout: 5))
        app.buttons["批准一次"].tap()
        app.buttons["停止"].tap()
        XCTAssertTrue(app.buttons["发送"].waitForExistence(timeout: 5))
    }
    func testRejectsHTTPWithoutOpeningNetworkConnection() {
        let app = launch("connection")
        let field = app.textFields["relay-address"]
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.tap(); field.typeText("http://example.test/remote#000102030405060708090a0b0c0d0e0f")
        app.buttons["connect"].tap()
        XCTAssertTrue(app.staticTexts["workspace-error"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["connect"].exists)
    }

    func testRunRowsExposeStatusAndExpandWithoutSending() {
        let app = launch("appearance")
        openConversation(app)
        let transcript = app.scrollViews["conversation-transcript"]
        let failedRun = app.buttons["run-merge"]
        for _ in 0..<6 {
            if failedRun.isHittable { break }
            transcript.swipeDown()
        }
        XCTAssertTrue(failedRun.isHittable)
        XCTAssertTrue(failedRun.label.contains("失败"))
        XCTAssertEqual(failedRun.value as? String, "已折叠")
        XCTAssertGreaterThanOrEqual(failedRun.frame.height, 44)
        failedRun.tap()
        XCTAssertEqual(failedRun.value as? String, "已展开")
        XCTAssertTrue(app.staticTexts["$ samtools merge merged.bam a.bam b.bam"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.staticTexts["Input file a.bam was not found."].exists)
        failedRun.tap()
        XCTAssertEqual(failedRun.value as? String, "已折叠")
        XCTAssertFalse(app.staticTexts["Input file a.bam was not found."].exists)
        XCTAssertFalse(app.buttons["send-message"].isEnabled)
    }

    func testExecutionProcessCollapsesPathsAndKeepsFinalAnswerVisible() {
        verifyProcessGrouping(darkAndLarge: false)
    }

    func testExecutionProcessGroupingWithAccessibilityText() {
        verifyProcessGrouping(darkAndLarge: true)
    }

    private func verifyProcessGrouping(darkAndLarge: Bool) {
        let app = launch("process", darkAndLarge: darkAndLarge)
        openConversation(app)
        let group = app.buttons["process-group-1"]
        revealEarlier(group, in: app)
        XCTAssertEqual(group.value as? String, "已折叠")
        XCTAssertTrue(group.label.contains("5 步"))
        XCTAssertTrue(group.label.contains("5 个文件"))
        XCTAssertTrue(group.label.contains("1 项失败"))
        XCTAssertGreaterThanOrEqual(group.frame.height, 44)
        let path = app.staticTexts["analysis/en-expression/output/figures/OG_log2FC_heatmap.pdf"]
        XCTAssertFalse(path.exists)
        XCTAssertFalse(app.buttons["shell"].exists)
        XCTAssertTrue(app.staticTexts["表达分析完成"].exists)
        group.tap()
        revealEarlier(path, in: app)
        XCTAssertTrue(path.exists)
        let shell = app.buttons["shell"].firstMatch
        revealEarlier(shell, in: app)
        shell.tap()
        XCTAssertTrue(app.staticTexts["python heatmap.py"].waitForExistence(timeout: 3))
        XCTAssertTrue(app.staticTexts["Heatmap generated."].exists)
        shell.tap()
        XCTAssertEqual(group.value as? String, "已展开")
        revealEarlier(group, in: app)
        group.tap()
        XCTAssertFalse(path.exists)
        XCTAssertTrue(app.staticTexts["表达分析完成"].exists)
        XCTAssertFalse(app.buttons["send-message"].isEnabled)
    }

    func testLiveProcessKeepsCommentaryAndApprovalOutsideTheFold() {
        let app = launch("process-running")
        openConversation(app)
        XCTAssertTrue(app.buttons["批准一次"].waitForExistence(timeout: 5))
        let group = app.buttons["process-group-3"]
        revealEarlier(group, in: app)
        XCTAssertEqual(group.value as? String, "已折叠")
        XCTAssertTrue(group.label.contains("1 项失败"))
        XCTAssertFalse(app.staticTexts["analysis/en-expression/README.md"].exists)
        let commentary = app.staticTexts["正在整理表达数据并生成图片。"]
        revealEarlier(commentary, in: app)
        XCTAssertTrue(commentary.isHittable)
    }

    func testUsageIsCollapsedAndRawPayloadRequiresExplicitExpansion() {
        let app = launch("appearance")
        openConversation(app)
        let usage = app.buttons["usage-details"]
        revealEarlier(usage, in: app)
        XCTAssertEqual(usage.value as? String, "已折叠")
        XCTAssertFalse(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "ctx_tokens")).firstMatch.exists)
        usage.tap()
        XCTAssertTrue(app.descendants(matching: .any)["usage-input"].firstMatch.waitForExistence(timeout: 3))
        XCTAssertFalse(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "ctx_tokens")).firstMatch.exists)
        let raw = app.buttons["usage-raw"]
        if !raw.isHittable { app.scrollViews["conversation-transcript"].swipeUp() }
        raw.tap()
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "ctx_tokens")).firstMatch.waitForExistence(timeout: 3))
        XCTAssertFalse(app.buttons["send-message"].isEnabled)
    }

    func testMalformedUsageRemainsReadableInsideDetails() {
        let app = launch("usage-malformed")
        openConversation(app)
        let usage = app.buttons["usage-details"]
        revealEarlier(usage, in: app)
        XCTAssertFalse(app.staticTexts["invalid usage payload"].exists)
        usage.tap()
        XCTAssertTrue(app.staticTexts["invalid usage payload"].waitForExistence(timeout: 3))
    }

    func testCompletedRunsAreGroupedWhileFailureStaysVisible() {
        let app = launch("appearance")
        openConversation(app)
        let completed = app.buttons["completed-runs"]
        revealEarlier(completed, in: app)
        XCTAssertEqual(completed.value as? String, "已折叠")
        XCTAssertFalse(app.buttons["run-probe"].exists)
        XCTAssertTrue(app.buttons["run-merge"].exists)
        completed.tap()
        let success = app.buttons["run-probe"]
        if !success.isHittable { app.scrollViews["conversation-transcript"].swipeUp() }
        XCTAssertTrue(success.isHittable)
        success.tap()
        XCTAssertTrue(app.staticTexts["Gene coordinates found."].waitForExistence(timeout: 3))
        // A second-level run disclosure must not collapse its completed group.
        XCTAssertEqual(completed.value as? String, "已展开")
        revealEarlier(completed, in: app)
        completed.tap()
        XCTAssertFalse(app.buttons["run-probe"].exists)
        XCTAssertTrue(app.buttons["run-merge"].exists)
    }

    func testLongConversationTitleDetailsAndJumpToLatest() {
        verifyConversationNavigation(darkAndLarge: false)
    }

    func testLongConversationNavigationWithAccessibilityText() {
        verifyConversationNavigation(darkAndLarge: true)
    }

    private func verifyConversationNavigation(darkAndLarge: Bool) {
        let app = launch("interaction", darkAndLarge: darkAndLarge)
        let title = "/data1/home/research/project/PlantRootAtlas/跨物种表达分析"
        openConversation(app, title: title)
        let info = app.buttons["conversation-info"]
        XCTAssertTrue(info.isHittable)
        XCTAssertFalse(info.frame.intersects(app.buttons["conversation-files"].frame))
        info.tap()
        XCTAssertTrue(app.navigationBars["对话详情"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts[title].firstMatch.exists)
        // The new details sheet uses the same window-level Escape stack as files.
        app.typeKey(XCUIKeyboardKey.escape, modifierFlags: [])
        XCTAssertFalse(app.buttons["完成"].exists)
        XCTAssertTrue(info.isHittable)
        let transcript = app.scrollViews["conversation-transcript"]
        transcript.swipeDown()
        transcript.swipeDown()
        let jump = app.buttons["jump-to-latest"]
        XCTAssertTrue(jump.waitForExistence(timeout: 5))
        XCTAssertTrue(jump.isHittable)
        XCTAssertGreaterThanOrEqual(jump.frame.height, 44)
        jump.tap()
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: NSPredicate(format: "exists == false"), object: jump)], timeout: 5), .completed)
        XCTAssertTrue(app.buttons["follow-up-2"].isHittable)
        XCTAssertFalse(app.buttons["send-message"].isEnabled)
    }

    private func revealEarlier(_ element: XCUIElement, in app: XCUIApplication,
                               file: StaticString = #filePath, line: UInt = #line) {
        let transcript = app.scrollViews["conversation-transcript"]
        for _ in 0..<10 {
            if element.isHittable { break }
            transcript.swipeDown()
        }
        XCTAssertTrue(element.isHittable, file: file, line: line)
    }

    func testLongSuggestionsStayReadableAndOnlyFillDraft() {
        verifyLongSuggestion(darkAndLarge: false)
    }

    func testLongSuggestionsWithDarkAppearanceAndAccessibilityText() {
        verifyLongSuggestion(darkAndLarge: true)
    }

    func testDemoEntryFilesScriptedReplyAndExitWithoutRemoteSetup() {
        let app = launch("connection")
        XCTAssertTrue(app.buttons["enter-demo"].waitForExistence(timeout: 5))
        app.buttons["enter-demo"].tap()
        XCTAssertTrue(app.buttons["exit-demo"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["RNA-seq 质控 · 演示"].firstMatch.waitForExistence(timeout: 5))
        app.staticTexts["RNA-seq 质控 · 演示"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["样本质控与报告"].firstMatch.waitForExistence(timeout: 5))
        app.staticTexts["样本质控与报告"].firstMatch.tap()
        XCTAssertTrue(app.buttons["conversation-files"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["添加图片"].exists)
        app.buttons["conversation-files"].tap()
        XCTAssertTrue(app.staticTexts["results"].waitForExistence(timeout: 5))
        app.staticTexts["results"].tap()
        app.staticTexts["qc_report.md"].tap()
        XCTAssertTrue(app.staticTexts["RNA-seq 质控报告"].waitForExistence(timeout: 5))
        app.typeKey(XCUIKeyboardKey.escape, modifierFlags: [])
        XCTAssertTrue(app.buttons["follow-up-0"].waitForExistence(timeout: 5))
        app.buttons["follow-up-0"].tap()
        app.buttons["send-message"].tap()
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "本地预设回复 · 未调用 AI")).firstMatch.waitForExistence(timeout: 5))
        app.buttons["exit-demo"].tap()
        XCTAssertTrue(app.buttons["enter-demo"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["exit-demo"].exists)
        XCTAssertFalse(app.staticTexts["workspace-error"].exists)
    }

    func testDemoApprovalCanBeReset() {
        let app = launch("connection")
        app.buttons["enter-demo"].tap()
        XCTAssertTrue(app.staticTexts["RNA-seq 质控 · 演示"].firstMatch.waitForExistence(timeout: 5))
        app.staticTexts["RNA-seq 质控 · 演示"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["分析流程与审批"].firstMatch.waitForExistence(timeout: 5))
        app.staticTexts["分析流程与审批"].firstMatch.tap()
        XCTAssertTrue(app.buttons["批准一次"].waitForExistence(timeout: 5))
        app.buttons["批准一次"].tap()
        XCTAssertFalse(app.buttons["批准一次"].exists)
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "演示审批已批准")).firstMatch.waitForExistence(timeout: 5))
        app.buttons["exit-demo"].tap()
        app.buttons["enter-demo"].tap()
        XCTAssertTrue(app.staticTexts["RNA-seq 质控 · 演示"].firstMatch.waitForExistence(timeout: 5))
        app.staticTexts["RNA-seq 质控 · 演示"].firstMatch.tap()
        app.staticTexts["分析流程与审批"].firstMatch.tap()
        XCTAssertTrue(app.buttons["批准一次"].waitForExistence(timeout: 5))
    }

    func testDemoQuestionChoiceAndFreeformSubmitWithoutRemote() {
        verifyDemoQuestion(darkAndLarge: false)
    }

    func testDemoQuestionWithDarkAppearanceAndAccessibilityText() {
        verifyDemoQuestion(darkAndLarge: true)
    }

    private func verifyDemoQuestion(darkAndLarge: Bool) {
        let app = launch("connection", darkAndLarge: darkAndLarge)
        XCTAssertTrue(app.buttons["enter-demo"].waitForExistence(timeout: 5))
        app.buttons["enter-demo"].tap()
        XCTAssertTrue(app.staticTexts["RNA-seq 质控 · 演示"].firstMatch.waitForExistence(timeout: 5))
        app.staticTexts["RNA-seq 质控 · 演示"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["问题与选项"].firstMatch.waitForExistence(timeout: 5))
        app.staticTexts["问题与选项"].firstMatch.tap()
        let submit = app.buttons["question-submit"]
        XCTAssertTrue(submit.waitForExistence(timeout: 5))
        XCTAssertFalse(submit.isEnabled, "No option is silently preselected")
        XCTAssertFalse(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "End your turn here")).firstMatch.exists)
        let choice = app.buttons["question-option-2"]
        let scroll = app.scrollViews.firstMatch
        for _ in 0..<12 { if choice.isHittable { break }; scroll.swipeDown() }
        XCTAssertTrue(choice.isHittable)
        choice.tap()
        XCTAssertEqual(choice.value as? String, "已选择")
        choice.tap()
        XCTAssertFalse(submit.isEnabled, "A choice can be cleared before submitting")
        choice.tap()
        let note = app.descendants(matching: .any).matching(identifier: "question-freeform").firstMatch
        for _ in 0..<12 { if note.isHittable { break }; scroll.swipeUp() }
        XCTAssertTrue(note.isHittable)
        note.tap(); note.typeText("重点检查 Q30")
        for _ in 0..<12 { if submit.isHittable { break }; scroll.swipeUp() }
        XCTAssertTrue(submit.isEnabled)
        submit.tap()
        let reply = app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "已收到你的演示回答")).firstMatch
        XCTAssertTrue(reply.waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["question-submit"].exists)
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "补充说明：重点检查 Q30")).firstMatch.exists)
        for _ in 0..<12 {
            if app.staticTexts["question-status"].isHittable { break }
            scroll.swipeDown()
        }
        XCTAssertEqual(app.staticTexts["question-status"].label, "已回答")
        XCTAssertFalse(app.staticTexts["workspace-error"].exists)
    }

    func testDemoBannerDoesNotOverlapNavigationAcrossColumns() {
        verifyDemoBannerLayout(darkAndLarge: false)
    }

    func testDemoBannerDoesNotOverlapNavigationWithAccessibilityText() {
        verifyDemoBannerLayout(darkAndLarge: true)
    }

    private func verifyDemoBannerLayout(darkAndLarge: Bool) {
        let app = launch("connection", darkAndLarge: darkAndLarge)
        let entry = app.buttons["enter-demo"]
        XCTAssertTrue(entry.waitForExistence(timeout: 5))
        if !entry.isHittable { app.swipeUp() }
        entry.tap()
        assertDemoBannerAboveNavigation(app)
        XCTAssertTrue(app.buttons["connection-settings"].isHittable)
        XCTAssertTrue(app.buttons["inbox"].isHittable)
        app.buttons["connection-settings"].tap()
        XCTAssertTrue(app.buttons["reset-demo"].waitForExistence(timeout: 5))
        app.typeKey(XCUIKeyboardKey.escape, modifierFlags: [])
        assertDemoBannerAboveNavigation(app)
        let project = app.staticTexts["RNA-seq 质控 · 演示"].firstMatch
        XCTAssertTrue(project.waitForExistence(timeout: 5))
        project.tap()
        assertDemoBannerAboveNavigation(app)
        let conversation = app.staticTexts["样本质控与报告"].firstMatch
        XCTAssertTrue(conversation.waitForExistence(timeout: 5))
        conversation.tap()
        XCTAssertTrue(app.buttons["conversation-files"].waitForExistence(timeout: 5))
        assertDemoBannerAboveNavigation(app)
        XCTAssertTrue(app.buttons["conversation-files"].isHittable)
        app.buttons["exit-demo"].tap()
        XCTAssertTrue(app.buttons["enter-demo"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.otherElements["demo-banner"].exists)
    }

    private func assertDemoBannerAboveNavigation(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
        let banner = app.otherElements["demo-banner"].firstMatch
        XCTAssertTrue(banner.waitForExistence(timeout: 5), file: file, line: line)
        // Check geometry, not just existence: the regression left both sets of
        // controls present in accessibility while drawing them on top of each other.
        let separated = NSPredicate { _, _ in
            let bounds = app.windows.firstMatch.frame
            let bars = app.navigationBars.allElementsBoundByIndex.filter {
                !$0.frame.isEmpty && $0.frame.intersects(bounds)
            }
            return !bars.isEmpty && bars.allSatisfy { banner.frame.maxY <= $0.frame.minY + 1 }
        }
        let expectation = XCTNSPredicateExpectation(predicate: separated, object: app)
        XCTAssertEqual(XCTWaiter.wait(for: [expectation], timeout: 5), .completed, file: file, line: line)
        let exit = app.buttons["exit-demo"]
        XCTAssertTrue(exit.isHittable, file: file, line: line)
        XCTAssertGreaterThanOrEqual(exit.frame.height, 44, file: file, line: line)
        XCTAssertFalse(app.staticTexts["离线演示 · 合成数据"].frame.intersects(exit.frame), file: file, line: line)
    }

    private func verifyLongSuggestion(darkAndLarge: Bool) {
        let app = launch("appearance", darkAndLarge: darkAndLarge)
        openConversation(app)
        let suggestion = app.buttons["follow-up-2"]
        XCTAssertTrue(suggestion.waitForExistence(timeout: 5))
        let transcript = app.scrollViews["conversation-transcript"]
        for _ in 0..<6 {
            if suggestion.isHittable { break }
            transcript.swipeUp()
        }
        XCTAssertTrue(suggestion.isHittable)
        XCTAssertGreaterThanOrEqual(suggestion.frame.height, 44)
        XCTAssertGreaterThanOrEqual(suggestion.frame.minX, transcript.frame.minX)
        XCTAssertLessThanOrEqual(suggestion.frame.maxX, transcript.frame.maxX)
        let send = app.buttons["send-message"]
        XCTAssertGreaterThanOrEqual(send.frame.height, 44)
        XCTAssertGreaterThanOrEqual(send.frame.minX, transcript.frame.minX)
        XCTAssertLessThanOrEqual(send.frame.maxX, transcript.frame.maxX)
        let capture = XCTAttachment(screenshot: app.screenshot())
        capture.name = darkAndLarge ? "Conversation — Dark, accessibility text" : "Conversation — Light"
        capture.lifetime = .keepAlways
        add(capture)
        suggestion.tap()
        let draft = app.textFields["message-draft"].exists ? app.textFields["message-draft"] : app.textViews["message-draft"]
        XCTAssertEqual(draft.value as? String, "其余两个 xhc 样品（big14_1/small14_1）是否也需要加入对比图并做同样的 QC 汇总？")
        XCTAssertFalse(app.staticTexts["已收到离线测试消息。"].exists)
        XCTAssertTrue(app.buttons["send-message"].isEnabled)
    }
}
