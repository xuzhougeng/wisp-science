package science.wisp.android

import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private const val CODE = "0001-0203-0405-0607-0809-0a0b-0c0d-0e0f"

class QrScannerTest {
    @Test fun onlyDesktopLinksCountAsScans() {
        assertTrue(isWispLink("https://wisp-relay.example.workers.dev/remote#$CODE"))
        assertTrue(isWispLink("https://relay.example.test/wisp/remote#$CODE/p/project/s/session"))
        for (payload in listOf(
            "WIFI:T:WPA;S:lab;P:secret;;", "https://example.com/", "https://relay.example.test/remote",
            "http://relay.example.test/remote#$CODE", CODE, "", "not a url at all",
        )) assertFalse(isWispLink(payload), payload)
    }
}
