// Self-contained mock of the Tauri v2 webview globals. Passed to
// Playwright's `page.addInitScript`, so it runs in the page before the Leptos
// wasm boots and installs `window.__TAURI__` with canned invoke/listen data.
//
// Keep it dependency-free and closure-free: Playwright serializes the function
// source and runs it verbatim in the browser.
export function tauriMock(fixtures?: { xlsxBase64?: string; pptxBase64?: string }): void {
  class Channel {
    onmessage: ((message: any) => void) | null = null;
  }
  const mp4Base64 = "AAAAIGZ0eXBpc29tAAACAGlzb21pc28yYXZjMW1wNDEAAANdbW9vdgAAAGxtdmhkAAAAAAAAAAAAAAAAAAAD6AAAAHgAAQAAAQAAAAAAAAAAAAAAAAEAAAAAAAAAAAAAAAAAAAABAAAAAAAAAAAAAAAAAABAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgAAAod0cmFrAAAAXHRraGQAAAADAAAAAAAAAAAAAAABAAAAAAAAAHgAAAAAAAAAAAAAAAAAAAAAAAEAAAAAAAAAAAAAAAAAAAABAAAAAAAAAAAAAAAAAABAAAAAABAAAAAQAAAAAAAkZWR0cwAAABxlbHN0AAAAAAAAAAEAAAB4AAAEAAABAAAAAAH/bWRpYQAAACBtZGhkAAAAAAAAAAAAAAAAAAAyAAAABgBVxAAAAAAALWhkbHIAAAAAAAAAAHZpZGUAAAAAAAAAAAAAAABWaWRlb0hhbmRsZXIAAAABqm1pbmYAAAAUdm1oZAAAAAEAAAAAAAAAAAAAACRkaW5mAAAAHGRyZWYAAAAAAAAAAQAAAAx1cmwgAAAAAQAAAWpzdGJsAAAAvnN0c2QAAAAAAAAAAQAAAK5hdmMxAAAAAAAAAAEAAAAAAAAAAAAAAAAAAAAAABAAEABIAAAASAAAAAAAAAABAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAGP//AAAANGF2Y0MBZAAK/+EAF2dkAAqs2V7ARAAAAwAEAAADAMg8SJZYAQAGaOvjyyLA/fj4AAAAABBwYXNwAAAAAQAAAAEAAAAUYnRydAAAAAAAAL7iAAC+4gAAABhzdHRzAAAAAAAAAAEAAAADAAACAAAAABRzdHNzAAAAAAAAAAEAAAABAAAAKGN0dHMAAAAAAAAAAwAAAAEAAAQAAAAAAQAABgAAAAABAAACAAAAABxzdHNjAAAAAAAAAAEAAAABAAAAAwAAAAEAAAAgc3RzegAAAAAAAAAAAAAAAwAAAsUAAAAMAAAADAAAABRzdGNvAAAAAAAAAAEAAAONAAAAYnVkdGEAAABabWV0YQAAAAAAAAAhaGRscgAAAAAAAAAAbWRpcmFwcGwAAAAAAAAAAAAAAAAtaWxzdAAAACWpdG9vAAAAHWRhdGEAAAABAAAAAExhdmY1OC43Ni4xMDAAAAAIZnJlZQAAAuVtZGF0AAACrgYF//+q3EXpvebZSLeWLNgg2SPu73gyNjQgLSBjb3JlIDE2MyByMzA2MCA1ZGI2YWE2IC0gSC4yNjQvTVBFRy00IEFWQyBjb2RlYyAtIENvcHlsZWZ0IDIwMDMtMjAyMSAtIGh0dHA6Ly93d3cudmlkZW9sYW4ub3JnL3gyNjQuaHRtbCAtIG9wdGlvbnM6IGNhYmFjPTEgcmVmPTMgZGVibG9jaz0xOjA6MCBhbmFseXNlPTB4MzoweDExMyBtZT1oZXggc3VibWU9NyBwc3k9MSBwc3lfcmQ9MS4wMDowLjAwIG1peGVkX3JlZj0xIG1lX3JhbmdlPTE2IGNocm9tYV9tZT0xIHRyZWxsaXM9MSA4eDhkY3Q9MSBjcW09MCBkZWFkem9uZT0yMSwxMSBmYXN0X3Bza2lwPTEgY2hyb21hX3FwX29mZnNldD0tMiB0aHJlYWRzPTEgbG9va2FoZWFkX3RocmVhZHM9MSBzbGljZWRfdGhyZWFkcz0wIG5yPTAgZGVjaW1hdGU9MSBpbnRlcmxhY2VkPTAgYmx1cmF5X2NvbXBhdD0wIGNvbnN0cmFpbmVkX2ludHJhPTAgYmZyYW1lcz0zIGJfcHlyYW1pZD0yIGJfYWRhcHQ9MSBiX2JpYXM9MCBkaXJlY3Q9MSB3ZWlnaHRiPTEgb3Blbl9nb3A9MCB3ZWlnaHRwPTIga2V5aW50PTI1MCBrZXlpbnRfbWluPTI1IHNjZW5lY3V0PTQwIGludHJhX3JlZnJlc2g9MCByY19sb29rYWhlYWQ9NDAgcmM9Y3JmIG1idHJlZT0xIGNyZj0yMy4wIHFjb21wPTAuNjAgcXBtaW49MCBxcG1heD02OSBxcHN0ZXA9NCBpcF9yYXRpbz0xLjQwIGFxPTE6MS4wMACAAAAAD2WIhAAz//727L4FNhTIwQAAAAhBmiJsQr/+wAAAAAgBnkF5Cv/EgQ==";
  const pdfBase64 = "JVBERi0xLjQKJVdpc3AKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUiA0IDAgUl0gL0NvdW50IDIgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCA2MTIgNzkyXSAvUmVzb3VyY2VzIDw8IC9Gb250IDw8IC9GMSA3IDAgUiA+PiA+PiAvQ29udGVudHMgNSAwIFIgPj4KZW5kb2JqCjQgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCA2MTIgNzkyXSAvUmVzb3VyY2VzIDw8IC9Gb250IDw8IC9GMSA3IDAgUiA+PiA+PiAvQ29udGVudHMgNiAwIFIgPj4KZW5kb2JqCjUgMCBvYmoKPDwgL0xlbmd0aCA0OCA+PgpzdHJlYW0KQlQgL0YxIDI0IFRmIDcyIDcyMCBUZCAoUERGIHByZXZpZXcgd29ya3MpIFRqIEVUCmVuZHN0cmVhbQplbmRvYmoKNiAwIG9iago8PCAvTGVuZ3RoIDQ2ID4+CnN0cmVhbQpCVCAvRjEgMjQgVGYgNzIgNzIwIFRkIChTZWNvbmQgUERGIHBhZ2UpIFRqIEVUCmVuZHN0cmVhbQplbmRvYmoKNyAwIG9iago8PCAvVHlwZSAvRm9udCAvU3VidHlwZSAvVHlwZTEgL0Jhc2VGb250IC9IZWx2ZXRpY2EgPj4KZW5kb2JqCnhyZWYKMCA4CjAwMDAwMDAwMDAgNjU1MzUgZiAKMDAwMDAwMDAxNSAwMDAwIG4gCjAwMDAwMDAwNjQgMDAwMDAgbiAKMDAwMDAwMDEyNyAwMDAwMCBuIAowMDAwMDAwMjUzIDAwMDAwIG4gCjAwMDAwMDAzNzkgMDAwMDAgbiAKMDAwMDAwMDQ3NyAwMDAwMCBuIAowMDAwMDAwNTczIDAwMDAwIG4gCnRyYWlsZXIKPDwgL1NpemUgOCAvUm9vdCAxIDAgUiA+PgpzdGFydHhyZWYKNjQyCiUlRU9GCg==";
  // Real .docx (pandoc-built) with headings, a table, and OMML equations —
  // exercises the offline docx-preview render path (P3 / #274).
  const docxBase64 = "UEsDBBQAAggIACwE8VwIaLrOgwEAAI0HAAATAAAAW0NvbnRlbnRfVHlwZXNdLnhtbLWVy07DMBBFfyXKFiVuWSCE+lgAXUIlyge49qSNiD2WPenj75kkNEIIkpa2m0jOzJx7fWMro+nOFNEGfMjRjuNhOogjsAp1blfj+H0xS+7j6WS02DsIEbfaMI7XRO5BiKDWYGRI0YHlSobeSOKlXwkn1YdcgbgdDO6EQktgKaGKEU9GT5DJsqDoecevG1kej6PHpq+SGsfSuSJXkrgs6qr4ddBDETomN1b/sJd8WUt5su4J69yFmw4J1ETZaRqYZbkCjao0PJLiMisDd4OeMaTWeeXEfa4hmktPL9IwU2zRa7GF5RsQcfoh7U6lX7cCOo8KQmCeKdJv8HbHfzqxpVmC597L+2jR/S7C1cIIRydBfM6heQ7P9lFj+jUzVljIZQGX33iL7nTB83OPLghWO9sDVNdJg07YiANPOXTH3oor9P9I4HDHq+nTJctAaM7ecoM5Vr0567Qv4Bonveb267eEiztoK0bmtt+IQlN1XyGKA/mYC4hkka7xPVp060LU/9fJJ1BLAwQUAAIICAAsBPFct3ek7+cAAADSAgAACwAAAF9yZWxzLy5yZWxzrZJNTgMxDEavEnnf8RQKQqhpN6hSdwiVA1iJZyai+VHiQrk9ASGgqAxddBnn8/OT5fly77fqmXNxMWiYNi0oDiZaF3oNj5vV5AaWi/kDb0lqogwuFVVbQtEwiKRbxGIG9lSamDjUny5mT1KfucdE5ol6xou2vcb8kwGHTLW2GvLaTkFtXhOfwo5d5wzfRbPzHOTIiF+JSqbcs2h4idmi/Sw3FQsKj+vMzqnDe+Fg2U5Srv1ZHJdvp6pzX8sFKaVRpcvTlf7ePnoWsiSEJmYeF3pPjBpdnXNJZlck+n+MPjJfTnhwnIs3UEsDBBQAAggIACwE8VyQXDPzMwUAAA4bAAARAAAAd29yZC9kb2N1bWVudC54bWzNWc1y2zYQvucpMDy100ikVCWWNZYyjh0nh7TxxG7jKwiCEhySYAFQtHLqtJdcO32SptNbH6GTvkOepAsSoH5MKZJsVdGBIHaBbxf7Ayyooyc3cYTGVEjGk77TanoOognhAUuGfeeHy7NG13kyeHCU9wJOspgmCsGERPbyvjNSKu25riQjGmPZ5ClNgBdyEWMFXTF0cy6CVHBCpQS8OHLbnvfYjTFLHAMTrwPDw5ARemoUsCBqZEHEtiCCRljBwuWIpdKi8b6TiaRnoBoxI4JLHqoG4XGvRDGNnTFeNWMcR3Zc3vLWwNZGszPwOisLBM6XmDdlZAsEmKUyUS0vT7fAmHf9acl0ikjyeTAZPEDwKzr8bYzF2wuFhUJ5jwUQhS0H3hIc075z5YVht+OF1Dvwg+DRoRd2fb8THFB62PIDfNDptg8O2+2Og9wKMx3ox7komgs1iSjAjXHUd15QrCO7pUcfudWg4lFEdk+mmIDYVFBJxZg6g1MWhlRAyDAcIXqjGTpXEA/R2VWD0ChCWn/IIA2oClhRgq9c5OF0jdiXSmCiNlpDe4M1HBsByzVcLu6MCanOscBDgdPRBkLfUAQBELKIBnNyV89CdWNFKc3XwqE5kUYLQ18CZdxzV+FLxqY8zczugbBOY4lUzhHhScAMNQlQyDN44h3pINkwYbBpYNiWYQ8LFWKJicaZUG2iyxEFspSffv6dJlQMJ8jufFsrBtvVd7AF65eipwbPjty44BtK+TgvGqkmKC7jKS1DqGKpQb9uohrEC2R5kaW6pTNjyNwY1zBlOdCMaS+OKdiuBXSrldyfX2Ku2BgrKhHPBEr0Lhmxd4XBHxZhobRDeEAjFDIFQRJFPF+xfyzPzqewl17SGzWXl2ZFOmnnOiXvmlhXENjVqLD+WBhWuRfM5Fd2x4SYphxFRsKi/fOLRaq4s66a1LjKNYBTx/kzM9htx/nWcf6dImxuSWbMxw9LYmlWJe/eVPpmQaUEi4ltb5n20/vfCoC4F7H4Ja8cCHvLq7EwLFDlBQuo5UE1UTHSOoY7K2x+mdf3nMqtWrMtJmpam6jWD9u67Hq1y6pAtA6oE3SzhhxWv/TPiL9DxNRa5K+NNa2ziMl9d24j2XW9kI+gzLqP82hXhtnAV5/e/70qhUiV2fDKUmUTdRXk99vm5Ff1gelti/dwjXP531/vfjBvoNLXi0hrnejNzxfsz+Csrsp1d/WlxZsW9CAii5TcWT3/usT/X8t5XUMqnqJg5j4UTWyZSQM0hNISil5BkcxisA57B0SfQmWzwtDKjwZlY0p5P5pT9xL7EXXKuh94b4CuJqm+M2WKa4Pnfceb8l+CY4AY6kW+5sArbpIRNr3CRQXzhEdZnBiK5s8REv7iKZRpVe/HmV6hlue1PWu7Wd2fCxbo1yG0gFjq137csSpuRnfnMFUpRFTCdOBAoW9U4kk1xw4h5fNclEKWB8UJj1Nz/1wzGp6Ds297taCQXcuO+LB9drIv6WkDJmYrFu9OXbVrXc5eXR2392WIdvPb1r5kt2ij+0V44OLVVetgb1Zodvcmu0Mbj78IDzw/vjzu7MsKjVazs78EpI1Hn3VBeYZs+yVhtQLPpl9DJcERnP45U6Pi48ZPGQ4EVowgRUV8j1+ZPv6xRvm57L62m+pz8WJmNf1QT7/ZtvBeJufP3Ve/La+u/J3ht2b5khIFShfdghByrhKu6JSIUPVyiwATkiyGGtdU14U5KCajCzrzjbwGwwp0b0vUtKlaulf+C6Hf7D9bgwf/AVBLAwQUAAIICAAsBPFc7t6hhAQBAACzBAAAHAAAAHdvcmQvX3JlbHMvZG9jdW1lbnQueG1sLnJlbHO11N1OwyAUwPFXIefe0k6dixnbzbJkt1ofgNLTj1iggTN1by+6WTrjhTdcNv9y+oOQrrcfemBv6HxvjYAiy4GhUbbuTSvgpdzfrGC7WT/hICm84bt+9CwsMV5ARzQ+cu5Vh1r6zI5oQmms05LCo2v5KNWrbJEv8nzJ3XwGXM9k5WnE/0y0TdMr3Fl11Gjoj8HcHHWFLvCBHWoB7lCvgJXStUgCppiFucB4MoWn04B+IjxEwrkk/z4ShW1GwXImuLTUhnesnn8z7iNjllNLGmuolNWAk+MuOqaYWkFhbRTcRsF3OOci/VFYMpZmd3MxP4pLTK1QVn+liCgi4qdNBn7199l8AlBLAwQUAAIICAAsBPFchNmMI20AAAB8AAAAHQAAAHdvcmQvX3JlbHMvZm9vdG5vdGVzLnhtbC5yZWxzTYxBDgIhDEWvQrp3ii6MMcPMbg5g9AANViAOhVBiPL4sXf689/68fvNuPtw0FXFwnCwYFl+eSYKDx307XGBd5hvv1IehMVU1IxF1EHuvV0T1kTPpVCrLIK/SMvUxW8BK/k2B8WTtGdv/BxhcflBLAwQUAAIICAAsBPFcm8Y5X00BAAClBgAAEgAAAHdvcmQvbnVtYmVyaW5nLnhtbL2VwY6CMBCGX6XpfS0gAhLR7MXEzWazB/YBKlRs0hbSFnDfflsEjZ42TZADk87/z+QryQyb3YUz0BGpaC0y6C88CIgo6pKKKoM/+f4tgbvtpk9Fy49EmiwwBUKlfQbPWjcpQqo4E47Vom6IMNqplhxrc5QV6mtZNrIuiFKmkjMUeF6EOKYC2p74qLTEhf5qOXg4HcoMrtfeYBKKlkbtMMugZ553KwBkJd4yTT9JR1j+25DJNGSZzY421jGjURNMBzheZs/1VHBsGSP6bs7J5aaBe/qjmJKMnCZ78y1toMIy2nwG48Dw9ekZi2r4iMvoyotGNxq6PYP584P5YehCFsxPFviRC9nyBWRJ4kIWzk9mQFzIVvOThUunCYjmJ1t5ThMQv4AsdpqAZH6yKPznBKCHFT5ygeFt97lvdvfz1j/cNvu01NHgn+L1j7P9A1BLAwQUAAIICAAsBPFcQAIyAx0KAAD0aQAADwAAAHdvcmQvc3R5bGVzLnhtbO1dW3fbqBZ+P7/Cy+8d32W5q+ksx6lXs04nyand9hnJONZEFjoIJc38+gFdkYws0CVV2jYPDWxA3977Y7NBl7z78/vR7j1C7FnIueiP/hj2e9Ax0c5y7i/6X7brN3r/z/fvnt565NmGXo+2dry3+KJ/IMR9Oxh45gEegfcHcqFDZXuEj4DQIr4foP3eMuEVMv0jdMhgPBxqAwxtQOiVvIPlev1otCeZ0Z4Q3rkYmdDzKLSjHY53BJbTf/+fXo8i3CHzCu6BbxOP1QR1+A5HdWFVXBmXwvIaOcTrPb0FnmlZWwoBXvSPloPwx6XjWX0qMT3CVV9au6AWAo8sPQvwXT5EdUx+YN0F4w0yl/f+oU0fgX3RH09PZSuvWGoD555KDQrnog/wm82SR3XR/+fwZnXTj/tD582XDTfEuwFniqiQMRa9giuyn5uzn+cC0wqQgD2BlBvUNbnruPx1soOympzrAtUIJc0mJB21P/IdQgeea0ydHdx/QuYD3G0IbXbRH0aV/1sHhEkrNvBofbR2O+ikdV+u77CFsEWeuTrnYO3gtwN0vnhwF9QPIiAB78NWDB+dIqxPUHtNW96wK9qsijy7FIoLMLjHwD30Exs64AhjH8TNB4n0/yHm6HqDaOjcxZPLXaLd8xZ+J9IXZB16YY/0mgagWt46xaBsy3ngR2ADrA4AFwLP80LEipEemNuAdFrDqDjIsOKcCUzfI+gYECLvg7WFPXKX2EHWNEG3Htev0ECp1dMmDi2fk0v59YxSK3SkBpR3dNJeSQtlJ0403oesJO/CRLetRWworVnUWoG+Zb7h6R2Mnue2b8UxIm42Gp43G10/d3Hb4XA5Gq6vFn15s4ZTg+KCLM4nhc8+IwbwCeLjKQuHDqE6+cDeRCPx0r/NGIlJYyjEIh/lF8LCZRD8LVwGWXXRMshkhcsgNx6POV0EZ1peki6BsyzfcOWQwXk9ZqFJS3QCUXMVs7CXJ0qOitEilsQUZtGzxFMhXRnFfnJvJr7b+AZRCiBph0LHnThDJYTE40tEkVGTUcTxj7lMzLIf7eRauRSMay0ZB6rwI+PrJMqFgGZFJBnrxSThZPWmfNZLkrM+7tTkxBfRsYwqFed+XQ+ayEaY2Yq1XLHCRZ8tPIFlg8qtxfLypZbsMmYL9tNlIix9ckBYOnrEzZuLHWVJ1wOE7g0bY5Cr/ERTAk9uMS/YVBZvKVUi8BXddUnbL2z823rnGGl4hEUhtbw47tWrlyDHwzRvY3Gqm9kETrInBZVzV6MooAzPuG/YsPuUPdfopqZNnwVu4rbuQ+HW/fxEaswT6WGIZdgWCuz8LH8gkulU1f4Fgwh9kF2mJQ96PkLAjmBH0nodwg69UaOk4vOWGFJ5mrs4bxO1LLcOb7MnThMtx0HkE7bL/pQmzbK8/qG7K1FWBkwWL9O8bHMAOzrE5TpJzIbr6Vwb5SdhLJ2emZ7TatMz4stYmcLjtik8VqSwlxwj830yZ8adJPyonPCjX5Xwk3Ex4TlZBcJPlAk/aZvwk9+ET3INVcK3sYeuS97m9sl58k6VyTttm7zTn5a80wx59VLuTl6au1a2tPKaZ3YFjs6UOTprm6Oz3xyNk8VXGV8rsFBTZqHWNgu1n5aFGRLm9yqnJJx1P1CqHaNX4OdcmZ/ztvk5/83PEIzwsYmXDpKtM1BXZqDeNgP1n5mB5zk3f50x8UpPGDmes586jFwoM3LRNiMXvy4j9a5GwVqcO3OjKHuCLvkIwsf4YL/JZxDS2wvSxHtFjx91+4C8nCDjSgQZt0CQcd3I9IuzprlT5nLWTCqxZtICayY/iDWdOErIMeBFHmnKnpQqMmDaAgOmr5QB3TjwLPf0rJKnZy14evZKPd0dX2qVfKm14EvtlfqyA6dv5W6eV3LzvAU3z1+pmzviSL2SI/UWHKm/Ukd24GSo3M2LSm5etODmxSt188se8V3ayHxQezeW9Sh7ObbKu58lPlI4rSt7pbb4udxwajnM73v2ouun4N3C8M1CuKdWn4aPzmDr/hCXTk/jSq2+Rog4iEAlw8ed1F9M5i2fvXTD1pdWXJ12ifZS/CvS8sUs0WkeFryVLw6ykqE86py+Gd7Lx2jpM3hF2Ftg8O9/kKAoxBi1LGRNID+dPnVvHhDDzhCBlq8D14aAd9+Dw7ank7N/2m4FbfsvkH1dkSC3pG+4JFKeCNqNhnq+pYEIXdVlxgzoVjoo818ee1TH3a2gpSCRuMPJgAHVP6OnPm8EM/e2JjEvEd5B7KW1WS0CV7LPnXC+TiCcdqZdH5e2dZ+wIBwor08CI9Ikwl4xW6KzxXIs9k2XLcRH+bfCkm69sF/FRSAdp+2bWJIvINXLPXl1lC0pa8TStW0FXCUISXsFJ8p8YqRs1RlnVh1ZV1nNuCoIsaqWCjr1yu0laHGe0/W+oHJ9BPfKugSdlHWp9H2Xex/LvxcZN683Gc59miVUCO4UgSX9eqUQTxu06f7s/JNMk5IvCzW5483tpiro8hViAxDrqKRL3KlMlaJAde4uJvs0j+MhG3jJjUi+quCuUVP3BjfQZJy78Y8GVPjsQdirF3dTMYj0zukz3EMMHRNKw0p2TlzXmr56hJhkcibPd2lSZWLLJSouSE+PqCKYUVr+6CjtUVOZknsa4bMKa310eVVJte3tKjqRkl/vble9pE+hdqKnUdTPWCZNb27jxIL7IBMtzBZxoejrTKdPOi0kU5Mf+vyDEaPNbd6M9IZ1TlLtHtpEm60XZffQOP7liVYS8JCPTQpmR3fO+agWiHqBjF79XGKQW5ayi0ogZk6k/7HPUn6jsOKWaL8PGkRuTjU6USiNCucV+i98ZhfZoocThTiRUB8BbBzCNsJS7MDoZHg+DN/Ej13SAPorQMCW9hDB52WK+DPAFxT2sHHg0PwKbCHsRFIH9HQIqL0bBn1JodyIMCeCzkFe2wgQEeRE0DnILHkjwBGi5mV1gOv6sHlOs2sKQUf19Sw9p7ZuGPDGhaYF7CLcOXH34BNMMwkh8kTSOdDxtYvBn7bonBIRM844IN+gjgqGoWm63rAK10cXYWGMSSWNrPp6C4EGHdk3v8UBMhEporcE6DVKHaPxpT/6ZHnwpXJhBpBv0IAmBhiPxqOGNVk6dLtcqEZWWo1LL+aUiDdfC1awjLTjqtzSjZFQi0RQJxi1ksavfccsohEvqwVcG+tz2PRqBrDFztyF6xgnqwN8tBjN52bzSSbByKZZ8FNBnsmLu7r9u3UhBgSJ2c7J6phfC/41vZPyLZtcC/nOierN0xYW3g/fCXS8gomaESpAjyDWhHaHYfT3O8R0yMtrJWTmHDRu2yWhyaLhE2EsyQjrQJ/vFsPxomHon+E9dftfAD+IF568/MXJce2Ef+ulgLk5ccfX928AOwXbDk7UcSWWNhTvPhJBA2vOft/CKccHjMURJhF0FXl4/iyCnkoqT834N+/9v1BLAwQUAAIICAAsBPFcvAATaRYBAABLAwAAEgAAAHdvcmQvZm9vdG5vdGVzLnhtbJ2SwW6DMAyGXwXlTkN3mCYE9FL1BbY9QBRCiUTiyDZke/ultGxd1U2oF0eW/X+/bKfafbghmwySBV+L7aYQmfEaWuuPtXh/O+QvYtdUsewA2AMbypLAUxlr0TOHUkrSvXGKNhCMT7UO0ClOKR5lBGwDgjZEiecG+VQUz9Ip68UF49ZgoOusNnvQozOeFwj3CwQfhaAZFKfBqbeBFhrUYkRfXlC5sxqBoONcgyvPlMuzKKb/FJMblr64LVawT0tbFGrNZC2q+Md6g9UPEJKKR/weL4YHGL9Pvz8XxfVPymLJn8HUQoNn68f5Eq8mKFQMKFLZtrUoZk04BTyFu82ZbCo5N8i5V/643HWkW5d8e2NDq9BXCTVfUEsDBBQAAggIACwE8VwfbgMT2QAAAHECAAARAAAAd29yZC9jb21tZW50cy54bWyd0cGKwyAQBuBXCd5T0z2URZr2UvYJtg8gxjRCxpEZE/fx19JY6KEl5CQy83844/H8B2M1W2KHvhX7XSMq6w12zt9acf39qb/F+XRMyiCA9ZGr3O9ZpVYMMQYlJZvBguYdButzrUcCHfOVbjIhdYHQWObMwSi/muYgQTsvFgbWMNj3ztgLmun+goLEoSC0FSE76pjn5sEFLhq2YiKvFqoGZwgZ+1jnDaiHshwlMX9KzDCWvrRvVtj3pZWEXjNZRzq9WW9wZoOQU3Gi53gpbDBev/7yKIpKnv4BUEsDBBQAAggIACwE8VzbVj2nHQEAAEcCAAARAAAAZG9jUHJvcHMvY29yZS54bWylks1OwzAQhF8l8j2xk6AWWWl6APUEEhJFIG6WvW2txj+yF9K+PW5CUyr1xs3rmf08a7tZHkyXfUOI2tkFKQtGMrDSKW23C/K2XuX3ZNk20nPpArwE5yGghpilNhu59AuyQ/Sc0ih3YEQsksMmceOCEZjKsKVeyL3YAq0Ym1EDKJRAQU/A3E9E8otUckL6r9ANACUpdGDAYqRlUdKLFyGYeLNhUP44jcajh5vWszi5D1FPxr7vi74erCl/ST+en16HUXNtIworgbSNkhw1dtA29LJMKxlAoAvj9lSk29zDsXdBxaRcVb8TjV5QWUrCx9xn5b1+eFyvSFuxapazeV7O14zxuubV3efpmKv+C9CkJ93ofxDPgDHx9W9ofwBQSwMEFAACCAgALATxXBp5JY2IAAAA1AAAABMAAABkb2NQcm9wcy9jdXN0b20ueG1snc7BCsIwEATQXwm5t4keRErTXsSzh+q9pJs2YLIhuy3696YIfoDHYYbHtP0rPMUGmTxGIw+1lgKixcnH2cj7cK3Osu/aW8YEmT2QKPtIRi7MqVGK7AJhpLrUsTQOcxi5xDwrdM5buKBdA0RWR61Pyq7EGKr04+TXazb+l5zQ7u/oMbzT7qnuA1BLAwQUAAIICAAsBPFcD58XAi0CAAAoBQAAEQAAAHdvcmQvc2V0dGluZ3MueG1snVRNb9swDL3vVwQ+L7GbpFkRNO2hXdZDOxRwursi07FQyRQoxZ7760d/KN7QoQt2svX4yCc+0r6+/Wn0pAJyCstNdDFLogmUEjNVHjbRy247vYpub67rtQPvGXMT5pdubTZR4b1dx7GTBRjhZmih5FiOZITnIx1izHMl4R7l0UDp43mSrGIOFtFQBDfRkcr1UGFqlCR0mPupRLPuk4dHyKD/lSXQwnOLrlDWhWpOn1OuDz2qPQlqQhOqDEWqj5qojA68+hytGimzhBKcY7ONfi9XXyRnuNbWidqxvSGaSb22QJK94AEnPOC4jYDZQ5Y2zoPZYuldj7I45qkXHjjrQMIYwZ5LDYJvwFtgQetuNQaoS3K+0fAsSth2rWyV9kDMrgQbnCTJcuBl+B39joR8fcIKBsUMcnHUfif2qUcbsr7Mwz0zEjUrfiOVPSCpN76r0KkVksHAXqz+wv4B5JX8iKuc1aIZq96PyV/5k2hOLfyZEAr/iy4Lwb2yFcMN7liEUAda58YdGks87eCkqOCZoFJQPyvpjwQ9zp9n5m4+TSbXcTgw6nkDoB3eoxj7g3L6koYbaErbNYEnYW3vgZDtIlxsouElOmHzgM1HbBGwxYgtA7YcscuAXY7YKmCrFtsfWFOrQ9FL7g/z4dip5ag11pA9NLypvGCvm+gd1PKKMV78jrcNZYJeu9ptJ+1hHsYGUhneg8bsR/dnQ1Ar51OwPCmPp5393AXj8a938wtQSwMEFAACCAgALATxXAFkTkZkAQAA1AIAABAAAABkb2NQcm9wcy9hcHAueG1snVLLTsMwELz3K6LciUt5qnJdIRDiAAipKZwte5NYOLZlGwR/z27ThiA4kdPuzM7sZhK+/uht8Q4xGe9W5XE1Lwtwymvj2lW5rW+PLsu1mPGn6APEbCAVKHBpVXY5hyVjSXXQy1Qh7ZBpfOxlxja2zDeNUXDj1VsPLrPFfH7O4COD06CPwmhYDo7L9/xfU+0V3Zee68+AfmJWFPzFR53E5QlnQ0XYppMRNGpFI20Czr4Bou9QHa1xr+m6k64FfRj7TdD4vXGQxPGCs6Ei7CqE5yFLJKo5PpxNsL3sNW1D7W9khsOGn+DeyRolM8kejIo++SYX9C4FOVeD8ThCEjwuSpVx14vJ3SZIhVedUQR/MiSpoQ+WVj5SxLbSPvecjSiNYDobUG/R5E+BS6ftzsFnaWvTgzhH4djs4lbSwjV+mDHuEfh5rji9OJseuaOfsGujDB1+Rc4m3UC2lD3hVMywGP8n8QVQSwMEFAACCAgALATxXJqsvQkXBwAAaiwAABUAAAB3b3JkL3RoZW1lL3RoZW1lMS54bWztWk1v2zYYvvdXELq7lmRLtou6hT+btkkbNG6HHmmZthhTokDSSY2iwNCedhkwoBt2WIHddhiGFViBFbvsxwRosXU/YpQcO6Is0246tMaaBAgiks/D9335fpnW1euPAgKOEOOYhnXDumwaAIUeHeBwVDfu97qFqgG4gOEAEhqiujFF3Lh+7dJVeEX4KEBAwkN+BdYNX4joSrHIPTkM+WUaoVDODSkLoJCPbFQcMHgsaQNStE3TLQYQhwYIYSBZ7w6H2EOgF1Ma1y4BMOfvEPknFDweS0Y9wg68ZOc00pjNJysGY2v+lDzzKW8RBo4gqRty/wE97qFHwgAEciEn6oaZ/BjFBUdRIZEURKyjTNF1kx+VLkWQSGirdGzUX/CZHbtatrLS2Io0GninGv9md0/DoedJi1qrKSzHNau2SpEBLWh0ktQqVimXZlmakkaamtu0y3k0pSWassas3Vqn7eTRlJdonNU0DdNu1kp5NM4SjbuaptxpVOxOHo2bovEJDscaErdSrboqiQKRgCElO3qWmuualbbKoqLikUXYLQJxSEOxJhIDeEhZV65TdidQ4BCIaYSG0JO4RiQoB23MIwKnBohgSLkcNm3LkmFZNu3Fb8oLEiYEUzSZOY+vnotFB9xjOBJ145bc0EitffP69cnTVydPfz959uzk6a9gF498oSPYgeEoTfDup2/+efEl+Pu3H989/3YNkKeBb3/56u0ff260oVAk/u7l21cv33z/9V8/P9fhGgz207geDhAHd9AxuEcDaQTdlqjPzgnt+RCnoY1wxGEIY7AO1hG+ArszhQTqAE2kHsMDJhOzFnFjcqgodeCzicA6xG0/UBB7lJImZXoD3I7FSNtuEo7WyMUmacA9CI+0YrUyjtSZRDIusXaTlo8UVfaJ9Co4QiESIJ6jY4R0+IcYK+ezhz1GOR0K8BCDJsR6Q/ZwX+Sjd3AgD3qqlV26lGLRvQegSYl2wzY6UiEyaCHRboKIcgo34ETAQK8VDEgasguFr1XkYMo85eC4kM40QoSCzgBxrgXfZVNFpdtQpmy9Z+2RaaBCmMBjLWQXUpqGtOm45cMg0uuFQz8NusnHMlIg2KdCLx9VYzh+lgcLw/Ue9QAjcc4MdV8m3HxnjGcmTBuriKo5ZEqGEGm3a7BAKTgNhvWe2JyMlFDbRYjAYzhACNy/qQXSiOYrdsuX2XIHaS16C6ohEz+HiMsuPW6fdS6DuRI5B2hE14m6N81k1ikMA8jW7nVnrLpnp89kAtGGDfHGSmHBLM44a+S7ywP4fvvs+1Dx5fiZrwmbKQvPnQ4k+PBDwOj8YFkB39+iPUhQvnP2IAa72uIjsZN8bBzwCX6iJxiqiSZ7nHHLu9S9xh0tDjftaLeik5VN4ZsfXnzE7vVj9K1rE2a2W10LyPaoLcoG+P/RorbhJNxHshxfdKgXHepFh7pFHerarHTRl+aiL/rSi770s+5L1R50dl87v4s9u54N1t3ODjEhB2JK0C5X21kuE9qgK2fPRmfjCd/i4jjy5b+KMsVcrESOGEwGAaPiCyz8Ax9GUibLyOww4oosi1EQUS77aEOdWi1Udt2sS58Ee3Rw+qWCpX7lo1JCcbbQdFYvlF2/mC1zK7mrEovMBczoVYwVW6mrk8j33+mrU0PVt7SJvpX8VefX1zI/mcK1TRSuWh+u8Gwk4+Gx3PLDI4y/bnXKMyvIdCCT0CD2+Ex4zQNp+6JrYydST8nexPi18vZFl6KvLpuo+urSji9bJ/267YmvmiZqFNPYm2lcqW5lfCXFNadOxqxhbvEkITiW9aDkyG08GNWNIYGy7feCSO7H4+oOySisG55g2fjMrbsbVd6VtTdBR4yLNuT+DJysyoDjpkIgBggOZKpbcr7kHYIwR03LrpifhZ418/97nrOnHA9HwyHyRK6Xp6YyG89m5PrMfrmIj820dBB0Is104A+OQZ9M2D0oz9SpWPFZDzAXi4MfYJbKHmcHnqm4+flVeQslPw0nCyGJfHjaTmraqxndci5cqJJ1oxztV5gxM6x6Q3/U/XgfGN6LcelUU51DXheYLVGV5RK1ou5s+SeclN6aBkzR3dmsPNfyy/PGDd0nbdVSZtGooZiltKFZNu77tvHzUkqRFQln43ZuG/q0vASV9G9B6m4kHlh6sTQuBP1DmfbaaAgnRPDi6Sh6JBhszV99m5ei2cTZHskjmDBcNx6bTqPcsp1Wwaw6nUK5VDYLVadRKjQcp2R1HMtsN+0nZ7cwwg8sZyZQFwaYTE/fp03Gl96pDebXSZc9GhRpcqNTTMDJO7WWvfqdWoClGR/bHatsN+xWodW23ELZbruFaqXUKLRst203ZKlzu40nBjhKFlvNdrvbdeyC25LrymbDKTSapVbBrXaadtfqlNumXFw8M7S0wtzEc/sszH3t0r9QSwMEFAACCAgALATxXD5cYF3ZAQAAFAkAABIAAAB3b3JkL2ZvbnRUYWJsZS54bWztlMtuozAUhvd5Csv7KYaQ5qKQqjd2M4tR+wAOmGDJF+TjhObtxzgkQ5WWVkStNNLAAvP/xufw6beXNy9SoB0zwLVKcHhFMGIq0zlXmwQ/P6U/ZhiBpSqnQiuW4D0DfLMaLetFoZUF5D5XsDAJLq2tFkEAWckkhStdMeW8QhtJrXs1m0AXBc/Yg862kikbRIRcB4YJal1pKHkFuF2t/sxqtTZ5ZXTGAFyvUhzWk5QrvBoh1DaI6oWi0vV9W1kN3vFeRZUGFjp7R0WCSUTuCCGxex7vGAen2VlJDTB7mk06XkElF/ujBTUH6LgVt1l5NHfUcLoWrOMD3zh3C2uSYPcDhESzKT4oYVPIX+NWiU4KaZXxayXz6/jXcJ62StiZ4wsvgwObtzA9cckA/WI1+q0lVX3AInJNxmTioE3ceDwQmPFlhgF7bHg9pulfYPdOmc4md2fA5h8DSwcB87lCDxwqQff/8/URrnsq165J9JPaso9WE6pDuJqQDaV1abhI1A1X7ABG8Un5Dlp6azgzzX7sgzV1iOY+VBOPbhgsqXNm3qVV8BeW9+3D2/N9GJ8H68v2YRusfy5TjfK9maKCO1J9oFKfI39IXQDqkqPq7URF8fTLTvbjCFajP1BLAwQUAAIICAAsBPFchRxUzpwAAADHAAAAFAAAAHdvcmQvd2ViU2V0dGluZ3MueG1sXY47DsIwEET7nMJyT2woEIryEU3oIqTAAUyyJJZsb+S1Eo7PQkFBOfP0RlM2L+/ECpEshkrucy0FhAFHG6ZK3m/t7iSbOisD6WKDRw8pMSHBVqCC20rOKS2FUjTM4A3luEBg+sToTeIYJ7VhHJeIAxCx7J06aH1U3tgg60yI77hxDrdrdxHqV43YYerNCmfq2XPQWgcfXqq/O/UbUEsBAgAAFAACCAgALATxXAhous6DAQAAjQcAABMAAAAAAAAAAAAAAAAAAAAAAFtDb250ZW50X1R5cGVzXS54bWxQSwECAAAUAAIICAAsBPFct3ek7+cAAADSAgAACwAAAAAAAAAAAAAAAAC0AQAAX3JlbHMvLnJlbHNQSwECAAAUAAIICAAsBPFckFwz8zMFAAAOGwAAEQAAAAAAAAAAAAAAAADEAgAAd29yZC9kb2N1bWVudC54bWxQSwECAAAUAAIICAAsBPFc7t6hhAQBAACzBAAAHAAAAAAAAAAAAAAAAAAmCAAAd29yZC9fcmVscy9kb2N1bWVudC54bWwucmVsc1BLAQIAABQAAggIACwE8VyE2YwjbQAAAHwAAAAdAAAAAAAAAAAAAAAAAGQJAAB3b3JkL19yZWxzL2Zvb3Rub3Rlcy54bWwucmVsc1BLAQIAABQAAggIACwE8VybxjlfTQEAAKUGAAASAAAAAAAAAAAAAAAAAAwKAAB3b3JkL251bWJlcmluZy54bWxQSwECAAAUAAIICAAsBPFcQAIyAx0KAAD0aQAADwAAAAAAAAAAAAAAAACJCwAAd29yZC9zdHlsZXMueG1sUEsBAgAAFAACCAgALATxXLwAE2kWAQAASwMAABIAAAAAAAAAAAAAAAAA0xUAAHdvcmQvZm9vdG5vdGVzLnhtbFBLAQIAABQAAggIACwE8VwfbgMT2QAAAHECAAARAAAAAAAAAAAAAAAAABkXAAB3b3JkL2NvbW1lbnRzLnhtbFBLAQIAABQAAggIACwE8VzbVj2nHQEAAEcCAAARAAAAAAAAAAAAAAAAACEYAABkb2NQcm9wcy9jb3JlLnhtbFBLAQIAABQAAggIACwE8VwaeSWNiAAAANQAAAATAAAAAAAAAAAAAAAAAG0ZAABkb2NQcm9wcy9jdXN0b20ueG1sUEsBAgAAFAACCAgALATxXA+fFwItAgAAKAUAABEAAAAAAAAAAAAAAAAAJhoAAHdvcmQvc2V0dGluZ3MueG1sUEsBAgAAFAACCAgALATxXAFkTkZkAQAA1AIAABAAAAAAAAAAAAAAAAAAghwAAGRvY1Byb3BzL2FwcC54bWxQSwECAAAUAAIICAAsBPFcmqy9CRcHAABqLAAAFQAAAAAAAAAAAAAAAAAUHgAAd29yZC90aGVtZS90aGVtZTEueG1sUEsBAgAAFAACCAgALATxXD5cYF3ZAQAAFAkAABIAAAAAAAAAAAAAAAAAXiUAAHdvcmQvZm9udFRhYmxlLnhtbFBLAQIAABQAAggIACwE8VyFHFTOnAAAAMcAAAAUAAAAAAAAAAAAAAAAAGcnAAB3b3JkL3dlYlNldHRpbmdzLnhtbFBLBQYAAAAAEAAQAAwEAAA1KAAAAAA=";
  const base64Bytes = (value: string) => Uint8Array.from(atob(value), (char) => char.charCodeAt(0));
  const xlsxBase64 = fixtures?.xlsxBase64 ?? "";
  const pptxBase64 = fixtures?.pptxBase64 ?? "";
  const listeners: Record<string, ((e: { payload: unknown }) => void) | undefined> = {};
  const windowListeners: Record<string, ((e: { payload: unknown }) => void) | undefined> = {};
  const emit = (event: string, payload: unknown) => {
    try {
      listeners[event]?.({ payload });
      windowListeners[event]?.({ payload });
    } catch {
      /* listener may not be registered yet */
    }
  };
  (window as any).__tauriEmit = emit;
  // Tests that exercise startup-time native events must wait until the WASM
  // side has completed its async `listen()` registration. Exposing readiness
  // avoids arbitrary sleeps and preserves the real event bus semantics: an
  // event emitted before registration is not queued.
  (window as any).__tauriListenerReady = (event: string) =>
    typeof listeners[String(event)] === "function"
      || typeof windowListeners[String(event)] === "function";
  (window as any).__tauriListenerScope = (event: string) => {
    const name = String(event);
    if (typeof windowListeners[name] === "function") return "window";
    if (typeof listeners[name] === "function") return "app";
    return null;
  };

  const demos = [
    { id: "manifest_esr1_01_datasets", title: "Help me find RNA-seq knockdown datasets involving ESR1" },
    { id: "manifest_esr1_02_samples", title: "What specific samples are included in GSE153250" },
    { id: "manifest_esr1_03_rnaseq", title: "Connect to the remote compute host, locate the FASTQ data for GSE153250" },
    { id: "manifest_esr1_04_downstream", title: "Based on the upstream Counts data from GSE153250, perform transcriptome" },
    { id: "manifest_esr1_05_hypotheses", title: "Based on the Counts data from our study, along with the differential e" },
    { id: "manifest_memory_01_long_context", title: "Long-context memory demo — GSE153250 ESR1-knockdown RNA-seq" },
  ];
  const runSummary = (run: any) => {
    const stdout = String(run.stdout_tail ?? "");
    const stderr = String(run.stderr_tail ?? "");
    return {
      id: run.id,
      frame_id: run.frame_id ?? null,
      context_id: run.context_id,
      title: run.title,
      kind: run.kind,
      status: run.status,
      created_at: run.created_at,
      started_at: run.started_at ?? null,
      ended_at: run.ended_at ?? null,
      exit_code: run.exit_code ?? null,
      remote_workdir: run.remote_workdir ?? null,
      timeout_secs: run.timeout_secs ?? null,
      last_polled_at: run.last_polled_at ?? null,
      last_poll_error: run.last_poll_error ?? null,
      progress_json: run.progress_json ?? "{}",
      harvested_at: run.harvested_at ?? null,
      cleaned_at: run.cleaned_at ?? null,
      cleanup_error: run.cleanup_error ?? null,
      output_fingerprint: `${stdout.length}:${stdout.slice(0, 64)}:${stdout.slice(-128)}|${stderr.length}:${stderr.slice(0, 64)}:${stderr.slice(-128)}`,
    };
  };
  const demoRunJson = JSON.stringify({
    id: "demo-run-001",
    frame_id: null,
    context_id: "ssh:remote-host",
    title: "Re-run pipeline with fixed STAR index",
    kind: "ssh_direct",
    status: "succeeded",
    command: "cd ~/workspace/GSE153250 && bash pipeline.sh",
    created_at: 1_700_000_000,
    started_at: 1_700_000_001,
    ended_at: 1_700_000_120,
    exit_code: 0,
    stdout_tail: "Pipeline finished: 38606 genes, 12 samples",
    stderr_tail: "",
    remote_workdir: null,
    timeout_secs: null,
    last_polled_at: 1_700_000_120,
    last_poll_error: null,
    progress_json: "{}",
    env_snapshot_json: "{}",
  });
  const demo = {
    id: "manifest_esr1_03_rnaseq",
    title: "ESR1 RNA-seq",
    request: "Connect to the remote compute host, locate the FASTQ data for GSE153250, keep only the siESR1 and siNT groups.",
    response: "## GSE153250 RNA-seq Upstream Analysis — Complete\n\nKept 12 samples: 6 siNT + 6 siESR1.",
    thinking: "Identify sample groups, download FASTQs, run the upstream pipeline.",
    items: [
      {
        role: "user",
        text: "Connect to the remote compute host, locate the FASTQ data for GSE153250, keep only the siESR1 and siNT groups.",
        tool_name: null,
        ok: null,
        input: "",
      },
      {
        role: "tool",
        text: demoRunJson,
        tool_name: "monitor_run",
        ok: true,
        input: "demo-run-001",
      },
      {
        role: "assistant",
        text: "## GSE153250 RNA-seq Upstream Analysis — Complete\n\nKept 12 samples: 6 siNT + 6 siESR1.",
        tool_name: null,
        ok: null,
        input: "",
      },
    ],
  };

  const project = {
    id: "default",
    name: "wisp-science",
    root: "/mock/root",
    skill_count: 12,
    mcp_server_count: 8,
    memory_file_count: 2,
    has_api_key: true,
  };
  let projectAgentContext = "";
  const projectNames: Record<string, string> = { default: project.name, other: "Other project" };
  const projectDescriptions: Record<string, string> = { default: "", other: "" };
  const projectAgentContexts: Record<string, string> = { default: "", other: "" };
  const query = new URLSearchParams(window.location.search);
  const mockPlanFlow = query.get("mockPlanFlow");
  const mockPublication = query.get("mockPublication");
  const mockLongPages = Number(query.get("mockLongPages") ?? 0);
  const mockLongRows = Math.min(200, Math.max(20, Number(query.get("mockLongRows") ?? 20)));
  const mockLongRowBytes = Math.min(
    64 * 1024,
    Math.max(256, Number(query.get("mockLongRowBytes") ?? 256)),
  );
  const mockLongSession = query.get("mockLongSession") === "1" || mockLongPages > 0;
  const mockResourceSession = query.get("mockResourceSession") === "1";
  const mockMcpAppSession = query.get("mockMcpAppSession") === "1";
  const mockBrowserRestore = query.get("mockBrowserRestore") === "1";
  const mockOAuthPending = query.get("mockOAuthPending") === "1";
  const mockOnboarding = query.get("mockOnboarding") === "1";
  const mockSyncUnconfigured = query.get("mockSyncUnconfigured") === "1";
  const mockExplorationFlow = query.get("mockExplorations") === "1";
  const mockOtherExplorationSession = query.get("mockOtherExplorationSession") === "1";
  const mockBranchFlow = query.get("mockBranches") === "1";
  const mockHistoricalExploration = query.get("mockHistoricalExploration") === "1";
  let mockLocale = query.get("mockLocale") === "zh" ? "zh" : "en";
  const mockSessions: any[] = mockExplorationFlow
    ? [
        { id: "exploration-mainline", title: "Mainline analysis", ts: 2100, running: false },
        ...(mockOtherExplorationSession
          ? [{ id: "session-b", title: "Unrelated analysis", ts: 2095, running: false }]
          : []),
        ...(mockBranchFlow
          ? [{ id: "conversation-branch", title: "Branch: alternate analysis", ts: 2090, running: false, branched_from: "exploration-mainline", branch_state: "active" }]
          : []),
      ]
    : mockBranchFlow
      ? [
          { id: "conversation-main", title: "Main analysis", ts: 2100, running: false },
          { id: "conversation-branch", title: "Branch: alternate analysis", ts: 2090, running: false, branched_from: "conversation-main", branch_state: "active" },
          { id: "conversation-branch-b", title: "Method B", ts: 2080, running: false, branched_from: "conversation-main", branch_state: "active" },
          { id: "conversation-branch-c", title: "Method C", ts: 2070, running: false, branched_from: "conversation-main", branch_state: "active" },
        ]
    : mockPlanFlow
    ? [{ id: "s1", title: "Plan mode regression", ts: 2000, running: false }]
    : mockPublication
      ? [{ id: "publication-session", title: "Publication evidence", ts: 2000, running: false }]
    : query.get("mockManySessions") === "1"
    ? Array.from({ length: 101 }, (_, index) => ({
        id: `session-${String(index + 1).padStart(3, "0")}`,
        title: `Paged session ${index + 1}`,
        ts: 2000 - index,
        running: false,
      }))
    : mockLongSession
      ? [{ id: "long-session", title: "Long transcript", ts: 2000, running: false }]
      : mockMcpAppSession
        ? [{ id: "mcp-app-session", title: "Saved MCP App", ts: 2000, running: false }]
      : mockBrowserRestore
        ? [{ id: "browser-restore-session", title: "Live PubMed retrieval", ts: 2000, running: false }]
      : query.has("mockAgentWorkflow")
        ? [{ id: "s-current", title: "Agent workflow conversation", ts: 2000, running: false }]
        : query.get("mockSessionModels") === "1"
          ? [
              { id: "s-model-a", title: "First model session", ts: 2000, running: false },
              {
                id: "s-model-b",
                title: "Second model session",
                ts: 1900,
                running: query.get("mockBackgroundApproval") === "1",
              },
            ]
      : [];
  if (query.get("mockNamedUnusedDraft") === "1") {
    mockSessions.unshift({
      id: "named-unused-draft",
      title: "Named unused draft",
      ts: 9000,
      running: false,
      has_user_turn: false,
    });
  }
  let activeMockFrame = mockExplorationFlow ? "exploration-mainline" : "";
  let mockBranchMergedSummary = "";
  let mockMainlineAdvanced = query.get("mockMainlineAdvanced") === "1";
  const makeMockExploration = (id: string, frameId: string, name: string, createdAt: number) => ({
    id,
    checkpoint_id: "checkpoint-shared",
    frame_id: frameId,
    name,
    status: "active",
    workspace_dir: `/mock/app-data/explorations/default/${id}/workspace`,
    workspace_backend: "copy",
    scope_generation: 1,
    warnings_json: "[]",
    created_at: createdAt,
    updated_at: createdAt,
  });
  let mockExplorations: any[] = mockExplorationFlow
    ? [
        { exploration: makeMockExploration("exploration-a", "exploration-frame-a", "Exploration A", 2001), source_frame_id: "exploration-mainline", checkpoint_user_index: 0, isolation_summary_json: '{"partial":false}' },
        { exploration: makeMockExploration("exploration-b", "exploration-frame-b", "Exploration B", 2002), source_frame_id: "exploration-mainline", checkpoint_user_index: 0, isolation_summary_json: '{"partial":true}' },
      ]
    : [];
  let mockExplorationRoundResolved = false;
  const explorationTranscript = (frameId: string) => {
    const suffix = frameId === "exploration-mainline" ? "Mainline result" : frameId.endsWith("-a") ? "Exploration A result" : frameId.endsWith("-b") ? "Exploration B result" : "New exploration result";
    const branches = frameId === "exploration-mainline" && mockBranchFlow
      ? mockSessions
          .filter((row) => row.branched_from === frameId)
          .map((row) => ({
            id: row.id,
            title: String(row.title).replace(/^Branch: /, ""),
            source_session_id: frameId,
            checkpoint_user_index: 0,
            checkpoint_kind: "after_response",
            merged: Boolean(row.branch_state === "merged"),
            merge_summary: row.branch_state === "merged" ? mockBranchMergedSummary : null,
          }))
      : [];
    if (mockHistoricalExploration && frameId === "exploration-mainline") {
      return {
        items: [
          { role: "user", text: "First method", tool_name: null, ok: null },
          { role: "assistant", text: "First result", tool_name: null, ok: null },
          { role: "user", text: "Legacy method", tool_name: null, ok: null },
          { role: "assistant", text: "Legacy result", tool_name: null, ok: null },
          { role: "user", text: "Latest method", tool_name: null, ok: null },
          { role: "assistant", text: suffix, tool_name: null, ok: null },
        ],
        next_before_seq: null,
        user_offset: 0,
        branches,
      };
    }
    return {
      items: [
        { role: "user", text: "Analyze the candidate method", tool_name: null, ok: null },
        { role: "assistant", text: suffix, tool_name: null, ok: null },
      ],
      next_before_seq: null,
      user_offset: 0,
      branches,
    };
  };
  const mockExplorationPreview = (id: string) => {
    const row = mockExplorations.find((item) => item.exploration.id === id);
    if (!row) throw new Error("Exploration not found");
    const blocked = mockMainlineAdvanced || row.source_frame_id !== "exploration-mainline";
    return {
      exploration: { ...row.exploration },
      diff: {
        explorationId: id,
        files: [{ path: `${id}/analysis.py`, kind: "modified", before: null, after: null }],
        artifacts: [{ logicalKey: `${id}/result`, beforeArtifactId: null, beforeVersionId: null, afterArtifactId: `${id}-artifact`, afterVersionId: `${id}-version` }],
        runs: [{
          id: `${id}-run`, frame_id: row.exploration.frame_id, context_id: "local", title: "Exploration validation", kind: "local", status: "succeeded", command: "python analysis.py", created_at: 2003, started_at: 2003, ended_at: 2004, exit_code: 0, stdout_tail: "ok", stderr_tail: "", remote_workdir: null, timeout_secs: null, last_polled_at: 2004, last_poll_error: null, progress_json: "{}", env_snapshot_json: "{}",
        }],
        decisions: [{ id: `${id}-decision`, kind: "decision", title: "Use candidate method", ref_id: null, metadata_json: "{}" }],
        researchEdges: [],
        externalResources: [{ id: `${id}-resource`, kind: "dataset", uri: "s3://example/reference", version: "1", checksum: "abc" }],
        externalEffects: [{ id: `${id}-effect`, effect_kind: "remote_run", recoverability: "not_recoverable", target_summary: "remote validation job", metadata_json: "{}", created_at: 2004 }],
      },
      mainlineChanges: {
        files: blocked ? [{ path: "mainline-notes.md", kind: "modified", before: null, after: null }] : [],
        artifactKeys: [], entityKeys: [], sourceMessageHead: blocked ? 5 : 4, sourceUiEventHead: 0, stateGeneration: blocked ? 2 : 1,
      },
      eligibility: {
        eligible: !blocked && row.exploration.status === "active",
        code: blocked ? "MainlineAdvanced" : null,
        reasons: blocked ? [{ code: "MainlineAdvanced", message: "The mainline no longer matches this exploration checkpoint." }] : [],
        expectedGuardHash: `guard-${id}`,
        manualResolutionAvailable: blocked && row.exploration.status === "active",
      },
    };
  };
  (window as any).__setMockMainlineAdvanced = (value: boolean) => { mockMainlineAdvanced = Boolean(value); };
  const mockCodexSessions = [
    { path: "/mock/.codex/sessions/2026/07/01/rollout-a.jsonl", session_id: "codex-a", title: "Fix the renderer crash", cwd: "/mock/project", message_count: 12, last_active_at: 1751340000, state: "new" },
    { path: "/mock/.codex/sessions/2026/07/02/rollout-b.jsonl", session_id: "codex-b", title: "Refactor session store", cwd: "/mock/other", message_count: 5, last_active_at: 1751426400, state: "imported" },
  ];
  const mockClaudeSessions = Array.from({ length: 27 }, (_, index) => ({
    path: `/mock/.claude/projects/mock-project/claude-${String(index + 1).padStart(2, "0")}.jsonl`,
    session_id: `claude-${String(index + 1).padStart(2, "0")}`,
    title: `Claude task ${String(index + 1).padStart(2, "0")}`,
    cwd: "/mock/project",
    message_count: index + 2,
    last_active_at: 1752000000 - index,
    state: "new",
  }));
  const mockFolders: Array<{ id: string; name: string }> = [];
  let activeProjectId = "default";
  let scratchOpen = false;
  let scratchSessionId: string | null = null;
  let terminalCounter = 0;
  let mockUpdateCheck = {
    current_version: "0.9.0",
    latest_version: "0.9.0",
    update_available: false,
    release_url: "https://github.com/xuzhougeng/wisp-science/releases",
    notes: "",
    install_supported: false,
    downloaded: false,
    downloading: false,
  };
  let mockUpdateCheckPending = false;
  let mockUpdateCheckError: string | null = null;
  let mockUpdateDownloadPending = false;
  let mockUpdateDownloadError: string | null = null;
  let resolveMockUpdateDownload: (() => void) | null = null;
  let mockInstallUpdateError: string | null = null;
  (window as any).__mockUpdateInstalled = false;
  let resolveMockOAuth: (() => void) | null = null;
  let mockPetEnabled = new URLSearchParams(window.location.search).get("mockPet") === "1";
  let mockPetDirectory = mockPetEnabled ? "C:\\Users\\tester\\.codex\\pets\\wispy" : "";
  (window as any).__petWindowVisible = false;
  let resolveMockUpdateCheck: (() => void) | null = null;
  const syncedProjects = new Set<string>();
  const nextProjectOpenDelayMs: Record<string, number> = {};
  let nextProbeDelayMs = 0;
  let nextSessionImportDelayMs = 0;
  const nextProjectTransferDelayMs: Record<string, number> = {};
  let failNextProjectOpenId: string | null = null;
  let failNextNewSession: string | null = null;
  (window as any).__delayNextProjectOpen = (projectId: string, milliseconds: number) => {
    nextProjectOpenDelayMs[String(projectId)] = Math.max(0, Number(milliseconds) || 0);
  };
  (window as any).__delayNextProbe = (milliseconds: number) => {
    nextProbeDelayMs = Math.max(0, Number(milliseconds) || 0);
  };
  (window as any).__delayNextSessionImport = (milliseconds: number) => {
    nextSessionImportDelayMs = Math.max(0, Number(milliseconds) || 0);
  };
  (window as any).__delayNextProjectTransfer = (direction: string, milliseconds: number) => {
    nextProjectTransferDelayMs[String(direction)] = Math.max(0, Number(milliseconds) || 0);
  };
  (window as any).__failNextProjectOpen = (projectId: string) => {
    failNextProjectOpenId = String(projectId);
  };
  (window as any).__failNextNewSession = (message?: string) => {
    failNextNewSession = String(message || "Project not found");
  };
  (window as any).__setMockUpdateCheck = (value: Record<string, unknown>) => {
    mockUpdateCheck = { ...mockUpdateCheck, ...(value ?? {}) };
  };
  (window as any).__setMockUpdateCheckPending = (pending: boolean) => {
    mockUpdateCheckPending = Boolean(pending);
  };
  (window as any).__setMockUpdateCheckError = (error: string | null) => {
    mockUpdateCheckError = error == null ? null : String(error);
  };
  (window as any).__setMockUpdateDownload = (
    value: { pending?: boolean; error?: string | null },
  ) => {
    mockUpdateDownloadPending = Boolean(value?.pending);
    mockUpdateDownloadError = value?.error == null ? null : String(value.error);
  };
  (window as any).__resolveMockUpdateDownload = () => {
    resolveMockUpdateDownload?.();
    resolveMockUpdateDownload = null;
  };
  (window as any).__setMockInstallUpdateError = (error: string | null) => {
    mockInstallUpdateError = error == null ? null : String(error);
  };
  (window as any).__resolveMockUpdateCheck = () => {
    resolveMockUpdateCheck?.();
    resolveMockUpdateCheck = null;
  };
  (window as any).__resolveMockOAuth = () => {
    resolveMockOAuth?.();
    resolveMockOAuth = null;
  };
  let skills = [
    { name: "remote-compute-ssh", description: "Submit SSH-direct research Runs", tags: ["compute"], scope: "bundled", enabled: true, builtin: true, dir: "/skills/remote-compute-ssh" },
    { name: "literature-review", description: "Find and synthesize literature", tags: ["literature"], scope: "bundled", enabled: true, builtin: true, dir: "/skills/literature-review" },
    { name: "paper-narrative", description: "Shape a paper story", tags: [], scope: "global", enabled: true, builtin: false, dir: "/home/me/.wisp/skills/paper-narrative" },
    { name: "social-note", description: "Write paste-ready social copy for Xiaohongshu, WeChat, or Twitter after the user chooses a platform", tags: ["share"], scope: "bundled", enabled: true, builtin: true, dir: "/skills/social-note" },
  ];
  let plugins = [
    {
      id: "motif-for-claude-science",
      version: "0.2.1",
      display_name: "Motif for Claude Science",
      description: "Interactive sequence workbench",
      author: "jvogan",
      license: "MIT",
      source_uri: "/downloads/motif.zip",
      archive_sha256: "a".repeat(64),
      trust_state: "checksum_verified",
      enabled: false,
      skill_count: 1,
      skill_names: ["motif-for-claude-science"],
      mcp_server_count: 1,
      commands: ["node"],
      runtime_status: "ready",
      runtime_errors: [],
    },
  ];
  let memoryEnabled = true;
  let autoReviewEnabled = false;
  let autoFailureAnalysis = {
    enabled: false,
    failure_rate_threshold: 30,
    minimum_failures: 2,
  };
  const lastMessageBySession: Record<string, string> = {};
  const sessionDelegationEnabled: Record<string, boolean> = {};
  const sessionPlanMode: Record<string, boolean> = {};
  const sessionFullPermission: Record<string, boolean> = {};
  const sessionAgentCompletion: Record<string, { policy: "inline" | "background"; auto_resume: boolean }> = {};
  let lastDelegationSessionId = "s-current";
  // Mutable workspace fixture lets live FileChanged events prove that open
  // previews re-read content written by an agent tool.
  let workspaceR = 'library(Seurat)\nin_dir <- "data"\nplot(1:3)\n';
  (window as any).__setMockWorkspaceR = (value: string) => { workspaceR = String(value); };
  // Editor saves land here; read_file serves them back like a real workspace.
  const savedWorkspaceFiles = new Map<string, string>();
  const FILE_NOW = Date.now();
  const workspaceMtimes: Record<string, number> = {
    data: FILE_NOW - 5 * 86_400_000,
    DEG: FILE_NOW - 1 * 86_400_000,
    "report.csv": FILE_NOW - 90_000,
    "manuscript.docx": FILE_NOW - 2 * 86_400_000,
  };
  let workspaceEntries = [
    { path: "data", is_dir: true, size: 0 },
    { path: "DEG", is_dir: true, size: 0 },
    { path: "DEG/scripts", is_dir: true, size: 0 },
    { path: "DEG/scripts/04_limma_deg.R", is_dir: false, size: 512 },
    { path: "DEG/output", is_dir: true, size: 0 },
    { path: "DEG/output/figures", is_dir: true, size: 0 },
    { path: "DEG/output/figures/volcano.png", is_dir: false, size: 4096 },
    { path: "DEG/output/tables", is_dir: true, size: 0 },
    { path: "DEG/output/tables/all_genes.tsv", is_dir: false, size: 8192 },
    { path: "report.csv", is_dir: false, size: 4096 },
    { path: "config.json", is_dir: false, size: 64 },
    { path: "model.pdb", is_dir: false, size: 256 },
    { path: "sequences.fasta", is_dir: false, size: 256 },
    { path: "analysis.R", is_dir: false, size: 128 },
    { path: "qc.py", is_dir: false, size: 96 },
    { path: "pixi.toml", is_dir: false, size: 64 },
    { path: "analysis.ipynb", is_dir: false, size: 4096 },
    { path: "analysis.unknown", is_dir: false, size: 128 },
    { path: "protocol.rtf", is_dir: false, size: 256 },
    { path: "manuscript.docx", is_dir: false, size: 11351 },
    { path: "office-preview.xlsx", is_dir: false, size: 3600 },
    { path: "office-preview.pptx", is_dir: false, size: 8600 },
  ].map((entry) => ({
    ...entry,
    modified_unix_millis: workspaceMtimes[entry.path] ?? FILE_NOW - 30 * 86_400_000,
  }));
  type MemoryFile = { name: string; preview: string; bytes: number };
  const memoryByProject: Record<string, MemoryFile[]> = {
    default: [{ name: "2026-07-01.md", preview: "User prefers DeepSeek.", bytes: 128 }],
    other: [{ name: "other-2026-07-02.md", preview: "Notes for other workspace.", bytes: 64 }],
  };
  let globalMemories = [{ id: "global-memory-existing", content: "Prefer SI units across projects." }];
  const memoryFilesFor = (projectId: string) => {
    const id = projectId || "default";
    if (!memoryByProject[id]) memoryByProject[id] = [];
    return memoryByProject[id];
  };
  const memoryProjectName = (projectId: string) =>
    projectId === "other" ? "Other project" : project.name;
  const memoryViewFor = (projectId: string) => {
    const id = projectId || activeProjectId || "default";
    return {
      enabled: memoryEnabled,
      project_id: id,
      project_name: memoryProjectName(id),
      today_file: "2026-07-04.md",
      files: memoryFilesFor(id),
      global_memories: globalMemories,
    };
  };
  // Tauri v2 binds command arguments by camelCase name only, so the mock must
  // reject snake_case here or it hides real "browsed project is ignored" bugs.
  const resolveMemoryProjectId = (_args: any, arg: (key: string) => any) => {
    const raw = arg("projectId");
    if (raw == null || String(raw) === "") return activeProjectId || "default";
    return String(raw);
  };
  let mockSpecialists: any[] = [
    { id: "reviewer", name: "Reviewer", icon: "review", color: "clay", description: "", instructions: "rubric", model_id: "", skills: [], connectors: [], builtin: true },
    { id: "reader", name: "Reader", icon: "search", color: "clay", description: "Searches project sessions", instructions: "reader rubric", model_id: "", skills: [], connectors: [], builtin: true },
    { id: "scientific_illustrator", name: "Scientific Illustrator", icon: "image", color: "clay", description: "Creates scientific figures", instructions: "illustrator rubric", model_id: "", skills: ["figure-composer", "figure-style"], connectors: [], builtin: true },
  ];
  let sessionSpecialists: Record<string, string> = {};
  let mockBrowserUrlFilters = { block: [] as { host: string; reason?: string }[], prefer: [] as { host: string; reason?: string }[] };
  let mockBrowserAutoLaunch = true;
  let mockBrowserAutoCloseTabs = false;
  let mockPendingBrowserTabCleanups: any[] = [];
  let mockQuickActions = [{
    id: "literature_research",
    name: "Research literature",
    description: "Prepare the selected passage for a literature-review turn in the current conversation.",
    icon: "search",
    context: "selection",
    workflow_template_id: "literature_evidence_review",
    enabled: true,
    sort_order: 0,
    builtin: true,
  }];
  const mockMethodSearchWorkflowTemplate = {
    id: "develop_computational_method",
    name: "Develop computational method",
    description: "Audit evidence and a baseline, freeze an evaluator contract, run a durable method search, then review verified finalists.",
    builtin: true,
    proposal: {
      goal: "Develop and independently verify a reusable computational method",
      context: "Supply project-local source, evaluator, data, metric, and guardrail details.",
      approval_policy: "review_all",
      tasks: [
        { id: "literature_methods", instruction: "Review relevant methods", depends_on: [], task_kind: "agent", run_activity: null, capabilities: ["literature_search"], skill_ids: ["literature-review"], specialist_id: null, output_schema: null, isolated: false, model_id: null, executor: null, budget: { max_tokens: 16000, max_tool_calls: 16, max_cost_microunits: null } },
        { id: "data_audit", instruction: "Audit validation data", depends_on: [], task_kind: "agent", run_activity: null, capabilities: ["project_read", "reasoning"], skill_ids: ["analysis-workflow"], specialist_id: null, output_schema: null, isolated: false, model_id: null, executor: null, budget: { max_tokens: 16000, max_tool_calls: 16, max_cost_microunits: null } },
        { id: "baseline_analysis", instruction: "Inspect the baseline", depends_on: [], task_kind: "agent", run_activity: null, capabilities: ["project_read", "reasoning"], skill_ids: ["analysis-workflow"], specialist_id: null, output_schema: null, isolated: false, model_id: null, executor: null, budget: { max_tokens: 16000, max_tool_calls: 16, max_cost_microunits: null } },
        {
          id: "prepare_contract",
          instruction: "Freeze and audit the evaluator contract",
          depends_on: ["literature_methods", "data_audit", "baseline_analysis"],
          task_kind: "agent",
          run_activity: null,
          capabilities: ["code_run"],
          skill_ids: ["analysis-workflow"],
          specialist_id: null,
          output_schema: {
            type: "object",
            required: ["method_search_spec_artifact_version_id"],
            properties: { method_search_spec_artifact_version_id: { type: "string" } },
          },
          isolated: false,
          model_id: null,
          executor: null,
          budget: { max_tokens: 16000, max_tool_calls: 16, max_cost_microunits: null },
        },
        {
          id: "method_search",
          instruction: "Run the durable method search",
          depends_on: ["prepare_contract"],
          task_kind: "run_activity",
          run_activity: {
            activity: "method_search",
            context_id: "local",
            input_task_id: "prepare_contract",
            spec_output_pointer: "method_search_spec_artifact_version_id",
            max_candidates: 20,
            max_wall_seconds: 14400,
            max_evaluator_seconds: 120,
            max_cost_microunits: 5000000,
          },
          capabilities: [],
          skill_ids: [],
          specialist_id: null,
          output_schema: null,
          isolated: false,
          model_id: null,
          executor: null,
          budget: null,
        },
        { id: "verify_finalists", instruction: "Review verified finalists", depends_on: ["method_search"], task_kind: "agent", run_activity: null, capabilities: ["project_read", "review"], skill_ids: [], specialist_id: null, output_schema: { type: "object" }, isolated: false, model_id: null, executor: null, budget: { max_tokens: 16000, max_tool_calls: 16, max_cost_microunits: null } },
        { id: "method_report", instruction: "Write the method report", depends_on: ["prepare_contract", "method_search", "verify_finalists"], task_kind: "agent", run_activity: null, capabilities: ["reasoning"], skill_ids: [], specialist_id: null, output_schema: { type: "object" }, isolated: false, model_id: null, executor: null, budget: { max_tokens: 16000, max_tool_calls: 16, max_cost_microunits: null } },
      ],
    },
  };
  let mockWorkflowTemplates: any[] = [{
    id: "literature_evidence_review",
    name: "Literature evidence review",
    description: "Parallel evidence searches followed by synthesis.",
    builtin: true,
    proposal: {
      goal: "Review the literature evidence for a selected passage",
      context: "",
      approval_policy: "auto_safe",
      tasks: [
        {
          id: "supporting_evidence",
          instruction: "Find supporting evidence",
          depends_on: [],
          capabilities: ["literature_search"],
          specialist_id: null,
          output_schema: null,
          isolated: false,
          model_id: null,
          executor: null,
          budget: null,
        },
        {
          id: "challenging_evidence",
          instruction: "Find challenging evidence",
          depends_on: [],
          capabilities: ["literature_search"],
          specialist_id: null,
          output_schema: null,
          isolated: false,
          model_id: null,
          executor: null,
          budget: null,
        },
        {
          id: "synthesize",
          instruction: "Synthesize the evidence",
          depends_on: ["supporting_evidence", "challenging_evidence"],
          capabilities: ["reasoning"],
          specialist_id: null,
          output_schema: null,
          isolated: false,
          model_id: null,
          executor: null,
          budget: null,
        },
      ],
    },
  }, {
    id: "roundtable",
    name: "Roundtable",
    description: "Two parallel perspectives cross-review each other before a neutral chair synthesis.",
    builtin: true,
    proposal: {
      goal: "Run a two-perspective roundtable and chair synthesis",
      context: "",
      approval_policy: "auto_safe",
      tasks: [
        {
          id: "seat_1_opening",
          instruction: "Give an evidence-focused opening position",
          depends_on: [],
          capabilities: ["reasoning"],
          specialist_id: null,
          output_schema: null,
          isolated: false,
          model_id: null,
          executor: null,
          budget: null,
        },
        {
          id: "seat_2_opening",
          instruction: "Give a critical opening position",
          depends_on: [],
          capabilities: ["reasoning"],
          specialist_id: null,
          output_schema: null,
          isolated: false,
          model_id: null,
          executor: null,
          budget: null,
        },
        {
          id: "seat_1_review",
          instruction: "Cross-review both opening positions",
          depends_on: ["seat_1_opening", "seat_2_opening"],
          capabilities: ["reasoning"],
          specialist_id: null,
          output_schema: null,
          isolated: false,
          model_id: null,
          executor: null,
          budget: null,
        },
        {
          id: "seat_2_review",
          instruction: "Cross-review both opening positions",
          depends_on: ["seat_1_opening", "seat_2_opening"],
          capabilities: ["reasoning"],
          specialist_id: null,
          output_schema: null,
          isolated: false,
          model_id: null,
          executor: null,
          budget: null,
        },
        {
          id: "chair_synthesis",
          instruction: "Synthesize the reviews without erasing disagreement",
          depends_on: ["seat_1_review", "seat_2_review"],
          capabilities: ["reasoning"],
          specialist_id: null,
          output_schema: null,
          isolated: false,
          model_id: null,
          executor: null,
          budget: null,
        },
      ],
    },
  }];
  mockWorkflowTemplates.push(mockMethodSearchWorkflowTemplate);
  const quickActionSessions: Record<string, string> = {};
  let mockModels = [
    {
      id: "default",
      label: "deepseek-v4-pro",
      provider: "openai",
      api_url: "https://api.deepseek.com",
      model: "deepseek-v4-pro",
      has_api_key: true,
      active: true,
      max_tokens: 4096,
      context_window: 128000,
      reasoning_effort: query.get("mockSessionModels") === "1" ? "low" : "",
      service_tier: query.get("mockFastDefault") === "1" ? "priority" : "",
      supports_vision: query.get("mockTextOnlyModel") !== "1",
      use_for_vision: query.get("mockTextOnlyModel") !== "1",
      use_for_image_generation: false,
      use_for_video_generation: false,
    },
    {
      id: "opus",
      label: "opus-4.8",
      provider: "anthropic",
      api_url: "https://api.anthropic.com",
      model: "opus-4.8",
      has_api_key: true,
      active: false,
      max_tokens: 4096,
      context_window: 200000,
      reasoning_effort: query.get("mockSessionModels") === "1" ? "max" : "",
      service_tier: "",
      supports_vision: true,
      use_for_vision: false,
      use_for_image_generation: false,
      use_for_video_generation: false,
    },
  ];
  const activeHttpModelId = () => mockModels.find((model) => model.active)?.id ?? mockModels[0]?.id ?? "";
  // Baked model catalog (mirrors src-tauri model_catalog): exact id match
  // within vendor namespaces, never prefix matching.
  type MockCatalogEntry = {
    context_window: number;
    max_tokens: number;
    input_limit: number | null;
    supports_vision: boolean;
    efforts: string[];
  };
  const mockCatalog: Record<string, Record<string, MockCatalogEntry>> = {
    deepseek: {
      "deepseek-v4-pro": { context_window: 1000000, max_tokens: 384000, input_limit: null, supports_vision: false, efforts: [] },
      "deepseek-v4-flash": { context_window: 1000000, max_tokens: 384000, input_limit: null, supports_vision: false, efforts: [] },
    },
    openai: {
      "gpt-5.6-luna": { context_window: 1050000, max_tokens: 128000, input_limit: null, supports_vision: true, efforts: ["none", "low", "medium", "high", "xhigh"] },
    },
    anthropic: {
      "claude-opus-4-8": { context_window: 1000000, max_tokens: 128000, input_limit: null, supports_vision: true, efforts: ["low", "medium", "high", "xhigh", "max"] },
    },
    "kimi-for-coding": {
      "k3-256k": { context_window: 262144, max_tokens: 131072, input_limit: null, supports_vision: true, efforts: ["low", "high", "max"] },
    },
  };
  const mockCatalogLookup = (provider: string, apiUrl: string, modelRaw: string): MockCatalogEntry | null => {
    const model = modelRaw.trim().toLowerCase();
    if (!model) return null;
    const host = apiUrl.trim().toLowerCase().replace(/^[^:]+:\/\//, "").split("/")[0].split(":")[0];
    const byHost: Record<string, string> = {
      "api.anthropic.com": "anthropic",
      "api.openai.com": "openai",
      "api.deepseek.com": "deepseek",
      "api.moonshot.ai": "moonshotai",
      "api.moonshot.cn": "moonshotai",
      "api.kimi.com": "kimi-for-coding",
      "open.bigmodel.cn": "zhipuai",
    };
    const candidates = [byHost[host], provider === "anthropic" ? "anthropic" : "openai"]
      .filter((ns): ns is string => Boolean(ns));
    const tail = model.split("/").pop() ?? model;
    for (const ns of [...new Set(candidates)]) {
      const table = mockCatalog[ns];
      const entry = table?.[model] ?? (tail !== model ? table?.[tail] : undefined);
      if (entry) return entry;
    }
    return null;
  };
  const sessionModels: Record<string, string> = query.get("mockSessionModels") === "1"
    ? { "s-model-a": "default", "s-model-b": "default" }
    : {};
  const sessionReasoningEfforts: Record<string, string> = {};
  const sessionServiceTiers: Record<string, string> = {};
  let mockAcpAgents = [
    { id: "acp-test", label: "Test ACP Agent", command: "fake-acp", args: ["--stdio"] },
  ];
  let mockAgentWorkflowCounter = 0;
  let mockAgentWorkflows: any[] = [];
  const mockDynamicAgentOptions = {
    capabilities: [
      { id: "reasoning", display_name: "Reasoning", description: "Reason without project tools.", risk: "read_only" },
      { id: "project_read", display_name: "Project read", description: "Read and search project files.", risk: "read_only" },
      { id: "project_write", display_name: "Project write", description: "Read and modify project files.", risk: "write" },
      { id: "code_run", display_name: "Code execution", description: "Run bounded project code.", risk: "execute" },
      { id: "review", display_name: "Review", description: "Inspect project evidence without modifying it.", risk: "read_only" },
      { id: "delegation", display_name: "Nested delegation", description: "Create one bounded child batch within root-wide limits.", risk: "read_only" },
    ],
    skills: [
      { id: "analysis-workflow", name: "analysis-workflow", scope: "bundled" },
      { id: "literature-review", name: "literature-review", scope: "bundled" },
    ],
    models: [
      { id: "default", external: false },
      { id: "opus", external: true },
    ],
    executors: [
      {
        id: "native",
        kind: "native",
        profile_id: null,
        display_name: "Native",
        available: true,
        supported_features: ["project_read", "project_write", "code_execution", "isolation", "delegation"],
      },
      {
        id: "acp:generic-acp",
        kind: "acp",
        profile_id: "generic-acp",
        display_name: "Generic ACP",
        available: true,
        supported_features: ["project_read", "project_write", "code_execution", "delegation"],
      },
    ],
  };
  const dynamicCapabilityTools: Record<string, string[]> = {
    reasoning: [],
    project_read: ["read", "search", "grep"],
    project_write: ["read", "search", "grep", "write", "edit"],
    code_run: ["read", "search", "grep", "run_in_context", "get_run", "cancel_run"],
    literature_search: ["literature_search"],
    external_research: ["web_search"],
    visualization: ["read", "search", "grep", "write", "edit", "python", "r"],
    review: ["read", "search", "grep"],
    delegation: ["delegate_tasks", "get_delegated_result"],
  };
  const normalizeDynamicProposal = (value: any) => ({
    goal: String(value?.goal ?? ""),
    context: String(value?.context ?? ""),
    approval_policy: value?.approval_policy === "auto_safe" ? "auto_safe" : "review_all",
    tasks: Array.isArray(value?.tasks) ? value.tasks.map((task: any, index: number) => ({
      id: String(task?.id ?? `task_${index + 1}`),
      instruction: String(task?.instruction ?? ""),
      depends_on: Array.isArray(task?.depends_on) ? task.depends_on.map(String) : [],
      capabilities: Array.isArray(task?.capabilities) ? task.capabilities.map(String) : ["reasoning"],
      specialist_id: task?.specialist_id ? String(task.specialist_id) : null,
      output_schema: task?.output_schema ?? null,
      isolated: Boolean(task?.isolated),
      model_id: task?.model_id ? String(task.model_id) : null,
      executor: task?.executor ?? null,
      budget: task?.budget ?? null,
    })) : [],
  });
  const resolveDynamicTask = (workflowId: string, task: any) => {
    const capabilities = task.capabilities as string[];
    const canWrite = capabilities.some((id) => ["project_write", "visualization"].includes(id));
    const canExecute = capabilities.some((id) => ["code_run", "visualization"].includes(id));
    const canAccessNetwork = capabilities.some((id) => ["literature_search", "external_research"].includes(id));
    const tools = [...new Set(capabilities.flatMap((id) => dynamicCapabilityTools[id] ?? []))];
    const approvalReasons = [
      ...(canWrite ? ["requires project write access"] : []),
      ...(canExecute ? ["executes bounded project code"] : []),
      ...(canAccessNetwork ? ["accesses configured network sources"] : []),
      ...(task.isolated ? ["uses a temporary Git worktree and conflict-checked cherry-pick"] : []),
      ...(task.model_id === "opus" ? ["uses an external model"] : []),
      ...(task.executor?.kind === "acp" ? [`uses ACP executor ${task.executor.profile_id}`] : []),
      ...(capabilities.includes("delegation") ? ["may create one bounded nested Agent batch"] : []),
    ];
    const specialist = mockSpecialists.find((item) => item.id === task.specialist_id);
    return {
      id: task.id,
      stored_step_id: `${workflowId}:${task.id}`,
      instruction: task.instruction,
      depends_on: task.depends_on,
      capabilities,
      specialist_id: specialist?.id ?? null,
      specialist_name: specialist?.name ?? null,
      executor: {
        kind: String(task.executor?.kind ?? "native"),
        profile_id: task.executor?.profile_id ?? null,
        model_id: task.executor?.kind === "acp" ? null : (task.model_id ?? "default"),
      },
      workspace_policy: task.isolated ? "isolated" : (canWrite || canExecute ? "serialized_mutation" : "shared_read_only"),
      merge_policy: task.isolated ? "automatic_cherry_pick" : (canWrite || canExecute ? "shared_serialized" : "not_applicable"),
      tools,
      can_write: canWrite,
      can_execute: canExecute,
      can_access_network: canAccessNetwork,
      budget: task.budget ?? { max_tokens: 8000, max_tool_calls: 16, max_cost_microunits: null },
      timeout_secs: 600,
      approval_reasons: approvalReasons,
      output_schema: task.output_schema,
      result: null,
    };
  };
  const dynamicResult = (task: any, status: string, overrides: Record<string, unknown> = {}) => ({
    status,
    summary: status === "succeeded" ? `Completed ${task.id}.` : null,
    error: status === "failed" ? `Mock failure in ${task.id}.` : status === "blocked" ? "Blocked by a failed dependency." : null,
    child_frame_id: status === "pending" ? null : `agent-child-${task.id}`,
    input_tokens: status === "succeeded" ? 900 : 0,
    output_tokens: status === "succeeded" ? 240 : 0,
    tool_calls: status === "succeeded" ? 3 : 0,
    cost_microunits: status === "succeeded" ? 19000 : 0,
    duration_secs: status === "running" ? null : 2,
    full_result_available: !["running", "pending"].includes(status),
    ...overrides,
  });
  const dynamicWorkflowSnapshot = (input: any, existingId?: string) => {
    const proposal = normalizeDynamicProposal(input);
    const id = existingId ?? `workflow-${++mockAgentWorkflowCounter}`;
    const tasks = proposal.tasks.map((task: any) => resolveDynamicTask(id, task));
    const approval_reasons = tasks.flatMap((task: any) =>
      task.approval_reasons.map((message: string) => ({ task_id: task.id, message }))
    );
    const requiresConfirmation = proposal.approval_policy === "review_all" || approval_reasons.length > 0;
    return {
      workflow: {
        id,
        project_id: "default",
        workspace_id: project.root,
        frame_id: lastDelegationSessionId,
        root_workflow_id: id,
        parent_attempt_id: null,
        depth: 0,
        name: proposal.goal,
        description: "Dynamic temporary-Agent workflow",
        goal: proposal.goal,
        mode: proposal.approval_policy === "auto_safe" ? "automatic" : "manual",
        status: "draft",
        max_parallel: 2,
        requires_confirmation: requiresConfirmation,
        plan_json: "{}",
        version: 1,
        enabled: true,
        approved_at: null,
        created_at: 1,
        updated_at: 1,
      },
      steps: [],
      attempts: [],
      delegation_enabled: sessionDelegationEnabled[lastDelegationSessionId] ?? false,
      approval_policy: proposal.approval_policy,
      dynamic: {
        schema_version: 2,
        approval_policy: proposal.approval_policy,
        editable_proposal: proposal,
        tasks,
        approval_reasons,
      },
    };
  };
  const executeMockDynamicWorkflow = async (snapshot: any) => {
    snapshot.workflow.status = "running";
    for (const task of snapshot.dynamic.tasks) {
      if (task.result?.status === "succeeded") continue;
      task.result = task.depends_on.length ? null : dynamicResult(task, "running");
    }
    const cancellationDemo = snapshot.workflow.goal.includes("CANCEL DEMO");
    await new Promise((resolve) => setTimeout(resolve, cancellationDemo ? 5_000 : 120));
    if (snapshot.workflow.status === "cancelled") {
      return { workflow_id: snapshot.workflow.id, status: "cancelled", steps: [] };
    }
    const partialDemo = snapshot.workflow.goal.includes("PARTIAL DEMO") && !snapshot.partialFailureRecorded;
    let failedTaskId: string | null = null;
    for (const task of snapshot.dynamic.tasks) {
      if (partialDemo && failedTaskId === null) {
        failedTaskId = task.id;
        snapshot.partialFailureRecorded = true;
        task.result = dynamicResult(task, "failed");
      } else if (task.result?.status === "succeeded") {
        continue;
      } else if (failedTaskId && task.depends_on.includes(failedTaskId)) {
        task.result = dynamicResult(task, "blocked", { child_frame_id: null });
      } else {
        task.result = dynamicResult(task, "succeeded");
      }
    }
    snapshot.workflow.status = partialDemo ? "failed" : "succeeded";
    snapshot.workflow.version += 2;
    return { workflow_id: snapshot.workflow.id, status: snapshot.workflow.status, steps: [] };
  };
  const seedMockAgentWorkflow = (kind: string) => {
    if (kind === "nested") {
      const root = dynamicWorkflowSnapshot({
        goal: "Root delegation batch",
        context: "Root-wide bounded context.",
        approval_policy: "auto_safe",
        tasks: [
          { id: "parent", instruction: "Delegate bounded evidence checks", depends_on: [], capabilities: ["reasoning", "delegation"], isolated: false },
        ],
      });
      root.workflow.status = "running";
      root.dynamic.tasks[0].result = dynamicResult(root.dynamic.tasks[0], "running");
      const nested = dynamicWorkflowSnapshot({
        goal: "Nested evidence batch",
        context: "Inherited bounded context.",
        approval_policy: "auto_safe",
        tasks: [
          { id: "parent/leaf", instruction: "Inspect one evidence branch", depends_on: [], capabilities: ["reasoning"], isolated: false },
        ],
      });
      nested.workflow.frame_id = "agent-child-parent";
      nested.workflow.root_workflow_id = root.workflow.id;
      nested.workflow.parent_attempt_id = "attempt-parent";
      nested.workflow.depth = 1;
      nested.workflow.status = "failed";
      nested.delegation_enabled = false;
      nested.dynamic.tasks[0].result = dynamicResult(nested.dynamic.tasks[0], "failed");
      mockAgentWorkflows = [nested, root, ...mockAgentWorkflows];
      return root.workflow.id;
    }
    const goal = kind === "partial" ? "PARTIAL DEMO dependency failure" : kind === "succeeded" ? "Completed dynamic research" : "Main Agent parallel research batch";
    const snapshot = dynamicWorkflowSnapshot({
      goal,
      context: "Main Agent supplied shared context.",
      approval_policy: "review_all",
      tasks: [
        { id: "research_a", instruction: "Analyze evidence A", depends_on: [], capabilities: ["project_read"], isolated: false },
        { id: "research_b", instruction: "Analyze evidence B", depends_on: [], capabilities: ["reasoning"], isolated: false },
        { id: "synthesize", instruction: "Synthesize both results", depends_on: ["research_a", "research_b"], capabilities: ["review"], isolated: false },
      ],
    });
    if (kind === "parallel") {
      snapshot.workflow.status = "running";
      snapshot.dynamic.tasks[0].result = dynamicResult(snapshot.dynamic.tasks[0], "running");
      snapshot.dynamic.tasks[1].result = dynamicResult(snapshot.dynamic.tasks[1], "running");
    } else if (kind === "partial") {
      snapshot.partialFailureRecorded = true;
      snapshot.workflow.status = "failed";
      snapshot.dynamic.tasks[0].result = dynamicResult(snapshot.dynamic.tasks[0], "failed");
      snapshot.dynamic.tasks[1].result = dynamicResult(snapshot.dynamic.tasks[1], "succeeded");
      snapshot.dynamic.tasks[2].result = dynamicResult(snapshot.dynamic.tasks[2], "blocked", { child_frame_id: null });
    } else if (kind === "succeeded") {
      snapshot.workflow.status = "succeeded";
      for (const task of snapshot.dynamic.tasks) task.result = dynamicResult(task, "succeeded");
    }
    mockAgentWorkflows = [snapshot, ...mockAgentWorkflows];
    return snapshot.workflow.id;
  };
  (window as any).__seedMockAgentWorkflow = seedMockAgentWorkflow;
  const initialAgentWorkflow = query.get("mockAgentWorkflow");
  if (initialAgentWorkflow) seedMockAgentWorkflow(initialAgentWorkflow);
  const otherAgentWorkflow = query.get("mockOtherAgentWorkflow");
  if (otherAgentWorkflow) {
    const currentSessionId = lastDelegationSessionId;
    lastDelegationSessionId = "s-other";
    seedMockAgentWorkflow(otherAgentWorkflow);
    lastDelegationSessionId = currentSessionId;
  }
  const acpBindings: Record<string, string> =
    mockPlanFlow === "acp" || mockPlanFlow === "compat" ? { s1: "acp-test" } : {};
  const acpPermissionFrames: Record<string, string> = {};
  const askUserFrames: Record<string, string> = {};
  const acpLongResolvers: Record<string, (value: string) => void> = {};
  const nativeConfirmResolvers: Record<string, (value: string) => void> = {};
  (window as any).__nativeConfirmPending = {};
  (window as any).__finishAcpLong = (frameId?: string) => {
    const ids = frameId ? [frameId] : Object.keys(acpLongResolvers);
    for (const id of ids) {
      acpLongResolvers[id]?.(id);
      delete acpLongResolvers[id];
    }
  };
  let mockCredentials: Record<string, boolean> = {
    openalex_api_key: false,
    infinisynapse_api_key: false,
    scimaster_api_key: false,
    ncbi_api_key: false,
    ncbi_email: false,
  };
  let mockCustomCredentials: Array<{
    id: string;
    name: string;
    envVar: string;
    present: boolean;
  }> = [];
  let nextCustomCredential = 1;
  const mockChannels = {
    feishu_enabled: false,
    feishu_bound: false,
    feishu_international: false,
    feishu_app_id: "",
    feishu_has_secret: false,
    feishu_state: "stopped",
    feishu_detail: "",
    feishu_owner_open_id: "",
    feishu_pending_owner_open_id: "ou_pending",
    weixin_enabled: false,
    weixin_bound: false,
    weixin_state: "stopped",
    weixin_detail: "",
    device: {
      enabled: false,
      mode: "lan",
      hasToken: false,
      state: "stopped",
      bindIpv4: "",
      port: 18766,
      url: null as string | null,
      detail: "",
    },
  };
  let mockDeviceToken = "";
  let mockDeviceTokenSequence = 0;
  let mockFeishuPollCount = 0;
  let mockApprovalGrants = [
    {
      scope: "global",
      kind: "command",
      target: "shell",
      label: "Shell commands",
    },
  ];
  let mockMcpConnections = [
    {
      id: "conn-wolai",
      name: "wolai_cmp",
      enabled: true,
      transport: {
        kind: "http",
        url: "https://api.wolai.com/v1/mcp/",
        headers: [],
        auth: "none",
      },
    },
  ];
  const mockMcpTools = [
    { name: "wolai_search", description: "Search Wolai pages", inputSchema: { type: "object", properties: {} } },
    { name: "wolai_create_page", description: "Create a Wolai page", inputSchema: { type: "object", properties: {} } },
  ];
  const executionContexts = [
    {
      id: "local",
      kind: "local",
      label: "Local machine",
      config_json: "{}",
      capabilities_json: "{\"os\":\"linux\",\"arch\":\"x86_64\",\"python\":\"3.12.1\"}",
      last_probe_at: 1783482000,
      last_probe_status: "ok",
      last_probe_error: null,
      created_at: 1783478400,
      updated_at: 1783482000,
    },
    {
      id: "ssh:gpu-server",
      kind: "ssh",
      label: "gpu-server",
      config_json: "{\"alias\":\"gpu-server\"}",
      capabilities_json: "{\"gpu_summary\":\"NVIDIA A100\",\"scheduler\":\"slurm\",\"python_executable\":\"/opt/python/bin/python\",\"rscript_executable\":\"/opt/R/bin/Rscript\",\"r_jsonlite\":true}",
      last_probe_at: 1783482300,
      last_probe_status: "ok",
      last_probe_error: null,
      created_at: 1783478400,
      updated_at: 1783482300,
    },
  ];
  (window as any).__mockExecutionContexts = executionContexts;
  const sessionExecutionContexts: Record<string, string[]> = {};
  const contextStoragePrefs: Record<
    string,
    { remote_data_root: string; remote_workdir_root: string; local_results_dir: string }
  > = {
    // Confirmed by default so unrelated attach flows do not open the
    // first-use storage dialog; the storage-prefs spec deletes this entry.
    "ssh:gpu-server": {
      remote_data_root: "~/wisp/demo-project/data",
      remote_workdir_root: ".wisp-science/runs",
      local_results_dir: "remote/gpu-server",
    },
  };
  (window as any).__mockStoragePrefs = contextStoragePrefs;
  const remoteStagingEntries: any[] = [
    {
      id: "stage-input-001",
      context_id: "ssh:gpu-server",
      run_id: "run-kinase-001",
      remote_path: "~/.wisp-science/runs/run-kinase-001/inputs/plates.csv",
      source: "run_input",
      run_status: "succeeded",
      size_bytes: 20480,
      created_at: 1783482605,
      removed_at: null,
      state: "active",
    },
    {
      id: "stage-old-upload",
      context_id: "ssh:gpu-server",
      run_id: null,
      remote_path: "~/wisp/demo-project/data/matrix.tsv",
      source: "transfer",
      run_status: null,
      size_bytes: 1048576,
      created_at: 1783482000,
      removed_at: null,
      state: "replaced",
    },
    {
      id: "stage-new-upload",
      context_id: "ssh:gpu-server",
      run_id: null,
      remote_path: "~/wisp/demo-project/data/matrix.tsv",
      source: "transfer",
      run_status: null,
      size_bytes: 2097152,
      created_at: 1783482500,
      removed_at: null,
      state: "orphan",
    },
  ];
  (window as any).__mockRemoteStaging = remoteStagingEntries;
  let projectRunRetention: {
    run_retention_days: number | null;
    failed_run_retention_days: number | null;
    orphan_file_retention_days: number | null;
  } = {
    run_retention_days: null,
    failed_run_retention_days: null,
    orphan_file_retention_days: null,
  };
  const runWorkspaceFiles: Record<string, Record<string, any[]>> = {
    "run-kinase-001": {
      "": [
        { path: "results", kind: "dir", size_bytes: 3221225472, file_count: 132481 },
        { path: "qc_table.tsv", kind: "file", size_bytes: 2048, file_count: null },
      ],
      results: [
        { path: "results/summary.tsv", kind: "file", size_bytes: 4096, file_count: null },
      ],
    },
  };
  const runReviewDismissed = new Set<string>();
  let defaultExecutionContext: string | null = null;
  let runtimeInfos: any[] = [
    {
      runtimeId: "runtime-python-local",
      generation: 1,
      key: { projectId: "default", contextId: "local", language: "python" },
      status: "ready",
      interpreter: "/mock/python",
      version: "3.12.1",
      processId: 1201,
      startedAtMs: Date.now() - 60_000,
      lastActivityAtMs: Date.now() - 5_000,
      residentMemoryBytes: 512 * 1024 * 1024,
      lastError: null,
    },
    {
      runtimeId: "runtime-r-local",
      generation: 2,
      key: { projectId: "default", contextId: "local", language: "r" },
      status: "dead",
      interpreter: "/usr/bin/Rscript",
      version: "4.4.1",
      processId: null,
      startedAtMs: Date.now() - 120_000,
      lastActivityAtMs: Date.now() - 30_000,
      residentMemoryBytes: null,
      lastError: "runtime process exited unexpectedly",
    },
    {
      runtimeId: "runtime-python-ssh",
      generation: 1,
      key: { projectId: "default", contextId: "ssh:gpu-server", language: "python" },
      status: "busy",
      interpreter: "/opt/python/bin/python",
      version: "3.11.9",
      processId: 2201,
      startedAtMs: Date.now() - 180_000,
      lastActivityAtMs: Date.now(),
      residentMemoryBytes: 10 * 1024 * 1024 * 1024,
      lastError: null,
    },
  ];
  const runs: any[] = [
    {
      id: "run-kinase-001",
      project_id: "default",
      frame_id: "s-complete",
      context_id: "ssh:gpu-server",
      title: "Kinase screen QC",
      kind: "ssh_direct",
      status: "succeeded",
      command: "python qc.py",
      script_path: null,
      input_refs_json: "[]",
      output_specs_json: "[]",
      created_at: 1783482600,
      started_at: 1783482605,
      ended_at: 1783482609,
      exit_code: 0,
      stdout_tail: "wrote qc table",
      stderr_tail: "",
      remote_workdir: "~/.wisp-science/runs/run-kinase-001",
      remote_handle_json: "{\"kind\":\"ssh_direct\"}",
      timeout_secs: 14400,
      last_polled_at: 1783482609,
      last_poll_error: null,
      env_snapshot_json: "{}",
    },
    {
      id: "run-local-002",
      project_id: "default",
      frame_id: "s-complete",
      context_id: "local",
      title: "Local normalization",
      kind: "command",
      status: "running",
      command: "python normalize.py",
      script_path: null,
      input_refs_json: "[]",
      output_specs_json: "[]",
      created_at: 1783482700,
      started_at: 1783482701,
      ended_at: null,
      exit_code: null,
      stdout_tail: "",
      stderr_tail: "",
      remote_workdir: null,
      remote_handle_json: null,
      timeout_secs: 300,
      last_polled_at: null,
      last_poll_error: null,
      env_snapshot_json: "{}",
    },
  ];
  if (query.get("mockLiveRunClock") === "1") {
    const run = runs.find((item) => item.id === "run-local-002");
    const now = Math.floor(Date.now() / 1000);
    Object.assign(run, {
      created_at: now - 11,
      started_at: now - 10,
      last_polled_at: now,
    });
  }
  if (query.get("mockMethodSearch") === "1") {
    runs.push({
      id: "method-search-001",
      project_id: "default",
      frame_id: "s-complete",
      context_id: "local",
      title: "Develop computational method",
      kind: "method_search",
      status: "draft",
      command: null,
      script_path: null,
      input_refs_json: "[]",
      output_specs_json: "[]",
      created_at: 1783482800,
      started_at: null,
      ended_at: null,
      exit_code: null,
      stdout_tail: "",
      stderr_tail: "",
      remote_workdir: null,
      remote_handle_json: null,
      timeout_secs: 14400,
      last_polled_at: null,
      last_poll_error: null,
      progress_json: JSON.stringify({
        schema: "wisp.method-search-progress.v1",
        phase: "awaiting_approval",
        baseline_primary: 0.5372,
        best_primary: 0.5717,
        candidate_count: 21,
        successful_count: 17,
        failed_count: 4,
        cost_microunits: 1300000,
        current_strategy: "diagnostic:residual_slice",
      }),
      env_snapshot_json: "{}",
    });
  }
  (window as any).__mockRuns = runs;
  (window as any).__mockRunWorkspaceFiles = runWorkspaceFiles;
  // Finish a pending MONITORRUN turn without changing the run's state: the
  // test drives the run's status itself, then ends the turn to observe the
  // deferred review prompt.
  (window as any).__finishMonitorRun = () => {
    if (!monitorRunFrameId) return;
    const frameId = monitorRunFrameId;
    const run = runs.find((item) => item.id === "run-local-002");
    emit("agent", { kind: "ToolResult", frame_id: frameId, name: "monitor_run", ok: true, content: JSON.stringify(run) });
    emit("agent", { kind: "Done", frame_id: frameId, stop_reason: "end_turn" });
    resolveMonitorRun?.(frameId);
    resolveMonitorRun = null;
    monitorRunFrameId = null;
  };
  const mockMethodSearchDetails = () => ({
    run: runs.find((item) => item.id === "method-search-001"),
    state: {
      run_id: "method-search-001",
      spec_artifact_version_id: "spec-version-001",
      spec_sha256: "a".repeat(64),
      activity_version: 1,
      checkpoint_json: "{}",
      control_state: "run",
      result_status: null,
      created_at: 1783482800,
      updated_at: 1783482800,
    },
    spec: {
      schema: "wisp.method-search.v1",
      objective: "Improve validation AUPRC without violating runtime limits.",
      target: {
        language: "python",
        source_artifact_version_id: "source-version-001",
        source_path: "analysis/model.py",
        symbol: "fit_model",
      },
      evaluator: {
        artifact_version_id: "evaluator-version-001",
        entry_path: "analysis/evaluate.py",
        repetitions: 3,
        timeout_seconds: 120,
        protocol: "wisp_evaluate_jsonl_v1",
      },
      metrics: {
        primary: "auprc",
        direction: "maximize",
        guardrails: [{ metric: "runtime_seconds", op: "lte", value: 120 }],
      },
      inputs: [],
      protected_paths: ["analysis/evaluate.py", "data/validation.csv"],
      constraints: ["Keep the target signature unchanged."],
      budget: {
        max_candidates: 20,
        max_wall_seconds: 14400,
        max_evaluator_seconds: 120,
        max_cost_microunits: 5000000,
      },
      final_verification: { artifact_version_id: "holdout-version-001", path: "data/holdout.csv", repetitions: 5 },
    },
    audit: {
      schema: "wisp.method-search-audit.v1",
      preparationId: "prepare-001",
      baseline: {
        repetitions: 3,
        successful_repetitions: 3,
        failure_rate: 0,
        median_primary: 0.5372,
        spread: 0.001,
        median_absolute_deviation: 0.0005,
        noise_floor: 0.002,
      },
      sentinelReachable: true,
      protectedFiles: [{ path: "analysis/evaluate.py", sha256: "b".repeat(64) }],
      targetSourceSha256: "c".repeat(64),
      evaluatorArtifactVersionId: "evaluator-version-001",
      findings: ["Baseline stable across three repetitions."],
    },
    auditArtifactVersionId: "audit-version-001",
    candidates: [
      { id: "candidate-0", run_id: "method-search-001", parent_candidate_id: null, sequence: 0, strategy_key: "baseline", family: "baseline", status: "succeeded", primary_score: 0.5372, utility: 0.5372, metrics_json: "{}", runtime_ms: 1000, source_sha256: "d".repeat(64), patch_sha256: "e".repeat(64), source_blob_id: "blob-0", patch_blob_id: null, changed_lines: 0, dependency_count: 0, rationale: "Frozen baseline", diagnostic_summary: null, error: null, created_at: 1783482800, finished_at: 1783482801 },
      { id: "candidate-21", run_id: "method-search-001", parent_candidate_id: "candidate-0", sequence: 21, strategy_key: "diagnostic:residual_slice", family: "ridge", status: "succeeded", primary_score: 0.5717, utility: 0.5717, metrics_json: "{}", runtime_ms: 900, source_sha256: "f".repeat(64), patch_sha256: "1".repeat(64), source_blob_id: "blob-21", patch_blob_id: "patch-21", changed_lines: 8, dependency_count: 0, rationale: "Use robust residual features.", diagnostic_summary: null, error: null, created_at: 1783482900, finished_at: 1783482901 },
    ],
    strategies: [{ run_id: "method-search-001", strategy_key: "diagnostic:residual_slice", category: "diagnostic", weight: 1.4, attempts: 4, improvements: 2, cumulative_reward: 3.2, summary: "Residual analysis", source_refs_json: "[]", updated_at: 1783482901 }],
    outputs: [{ id: "output-selected", run_id: "method-search-001", artifact_version_id: "selected-version-001", role: "selected_method", logical_output_key: "selected_method", source_path: "method-search/selected_method.py", created_at: 1783483000 }],
    activity: { attempt_id: "attempt-search", run_id: "method-search-001", activity: "method_search", state_json: "{}", created_at: 1783482800, updated_at: 1783482800 },
  });
  let monitorRunFrameId: string | null = null;
  let resolveMonitorRun: ((frameId: string) => void) | null = null;
  const artifacts = [
    { id: "art-tree", name: "nif3.treefile", kind: "text/treefile", path: "nif3.treefile", ts: Math.floor(Date.now() / 1000), project_id: "default", project_name: "wisp-science", session_id: "s-current", session_title: "Current analysis", origin: "output" },
    { id: "art-profile", name: "plddt_profile.png", kind: "image/png", path: "plddt_profile.png", ts: Math.floor(Date.now() / 1000), project_id: "default", project_name: "wisp-science", session_id: "s-old", session_title: "Older structure run", origin: "output" },
    { id: "art-counts", name: "counts.csv", kind: "text/csv", path: "counts.csv", ts: Math.floor(Date.now() / 1000), project_id: "other", project_name: "Other project", session_id: "s-other", session_title: "Cross-project counts", origin: "upload" },
    { id: "art-html", name: "dashboard.html", kind: "text/html", path: "dashboard.html", ts: Math.floor(Date.now() / 1000), project_id: "default", project_name: "wisp-science", session_id: "s-current", session_title: "Current analysis", origin: "output" },
    { id: "art-markdown", name: "analysis-report.md", kind: "text/markdown", path: "analysis-report.md", ts: Math.floor(Date.now() / 1000), project_id: "default", project_name: "wisp-science", session_id: "s-current", session_title: "Current analysis", origin: "output" },
  ];
  let libraryItems: any[] = [];
  const librarySummary = ({ base64: _base64, code, ...item }: any) => ({
    ...item,
    code_preview: String(code ?? "").slice(0, 512),
  });
  const libraryVersions: Record<string, any[]> = {};
  const researchGraph = {
    nodes: [
      { id: "d1", kind: "decision", title: "Use DESeq2 over edgeR", ref_id: null, metadata_json: JSON.stringify({ rationale: "Better fit for the replicate design" }) },
      { id: "p1", kind: "paper", title: "Love et al. 2014", ref_id: "10.1186/s13059-014-0550-8", metadata_json: JSON.stringify({ journal: "Genome Biology" }) },
      { id: "a1", kind: "data_asset", title: "counts.tsv", ref_id: "data/counts.tsv", metadata_json: JSON.stringify({ rows: 24567 }) },
      { id: "run:r1", kind: "run", title: "DESeq2 differential expression", ref_id: "r1", metadata_json: "{}" },
      { id: "artifact:h1", kind: "artifact", title: "deseq2_results.tsv", ref_id: "h1", metadata_json: "{}" },
    ],
    edges: [
      { source_id: "d1", target_id: "p1", relation: "cites", metadata_json: JSON.stringify({ confidence: "high", evidence: "Methods section" }) },
      { source_id: "d1", target_id: "a1", relation: "applies to", metadata_json: "{}" },
      { source_id: "run:r1", target_id: "artifact:h1", relation: "produced", metadata_json: "{}" },
    ],
  };
  let publicationRevisionId = "publication-revision-1";
  let publicationRevisionState = mockPublication === "frozen" ? "frozen" : "draft";
  const publicationItems = [
    {
      id: "publication-section-results",
      revision_id: publicationRevisionId,
      parent_item_id: null,
      kind: "section",
      title: "Results",
      content: "",
      ordinal: 0,
    },
    {
      id: "publication-claim-1",
      revision_id: publicationRevisionId,
      parent_item_id: "publication-section-results",
      kind: "claim",
      title: "Exhausted T cells expand after treatment",
      content: "",
      ordinal: 0,
    },
    {
      id: "publication-figure-2b",
      revision_id: publicationRevisionId,
      parent_item_id: "publication-section-results",
      kind: "figure",
      title: "Figure 2B",
      content: "",
      ordinal: 1,
    },
  ];
  let publicationBindings: any[] = mockPublication === "frozen"
    ? [{
        id: "publication-binding-frozen",
        revision_id: publicationRevisionId,
        item_id: "publication-figure-2b",
        source_kind: "artifact_version",
        source_id: "artifact-version-late-v4",
        purpose: "Figure 2B treatment comparison",
        supported_claim_item_id: "publication-claim-1",
        selection_state: "selected",
        review_state: "reviewed",
        reproduction_state: "not_run",
        visibility: "public",
        source_snapshot_json: JSON.stringify({
          capture_timing: "late",
          historical_content_unverified: true,
        }),
      }]
    : [];
  let publicationCapsuleBuilds: any[] = [];
  let publicationReproductionRuns: any[] = [];
  let publicationReproductionResults: any[] = [];
  const publicationRevision = () => ({
    id: publicationRevisionId,
    publication_id: "publication-paper-1",
    parent_revision_id: publicationRevisionId === "publication-revision-1" ? null : "publication-revision-1",
    revision_number: publicationRevisionId === "publication-revision-1" ? 1 : 2,
    label: publicationRevisionId === "publication-revision-1" ? "Submission" : "Revision 2",
    state: publicationRevisionState,
    capability_level: publicationRevisionState === "frozen" ? "traceable" : "archived",
    manifest_sha256: publicationRevisionState === "frozen" ? "a".repeat(64) : null,
    frozen_at: publicationRevisionState === "frozen" ? 1785480000 : null,
    published_at: null,
  });
  const publicationReadiness = () => ({
    revision_id: publicationRevisionId,
    target_visibility: "public",
    capability_level: "traceable",
    blockers: [],
    warnings: [{
      code: "historical_content_unverified",
      message: "Historical bytes were unavailable; evidence was captured at freeze time",
      binding_id: "publication-binding-frozen",
      source_id: "artifact-version-late-v4",
      waivable: true,
      waived: false,
      waiver: null,
      details: { original_source_id: "artifact-version-original-v3" },
    }],
    omissions: [],
    manifest_sha256: "a".repeat(64),
    can_freeze: true,
  });
  const publicationWorkspace = () => ({
    publications: [{
      id: "publication-paper-1",
      project_id: "default",
      title: "T-cell treatment response",
      description: "Submission evidence",
    }],
    publication: {
      id: "publication-paper-1",
      project_id: "default",
      title: "T-cell treatment response",
      description: "Submission evidence",
    },
    revisions: [publicationRevision()],
    revision: publicationRevision(),
    items: publicationItems.map((item) => ({ ...item, revision_id: publicationRevisionId })),
    item_links: [{
      source_item_id: "publication-figure-2b",
      target_item_id: "publication-claim-1",
      relation: "supports",
    }],
    bindings: publicationBindings,
    reviews: publicationBindings.length ? [{
      binding_id: publicationBindings[0].id,
      reviewer: "Scientist",
      method: "traceability_check",
      verified_at: 1785480000,
      result: "passed",
      report_json: "{}",
    }] : [],
    supersessions: [],
    waivers: [],
    readiness: publicationRevisionState === "frozen" ? publicationReadiness() : null,
    drift: publicationBindings.length ? [{
      binding_id: publicationBindings[0].id,
      bound_version_id: "artifact-version-late-v4",
      bound_version_number: 4,
      latest_version_id: "artifact-version-v5",
      latest_version_number: 5,
      has_drift: true,
    }] : [],
    lineage: publicationBindings.map((binding) => ({
      binding_id: binding.id,
      source_label: binding.source_kind === "run" ? "Kinase screen QC" : "plddt_profile.png",
      quality: "likely",
      bases: ["declared", "observed"],
      exact_version_id: binding.source_kind === "artifact_version" ? binding.source_id : null,
      version_number: binding.source_kind === "artifact_version" ? 4 : null,
      checksum: binding.source_kind === "artifact_version" ? "b".repeat(64) : null,
      capture_timing: binding.source_kind === "artifact_version" ? "late" : null,
      producing_run_id: binding.source_kind === "artifact_version" ? "run-kinase-001" : binding.source_id,
      run_input_count: 2,
      run_output_count: 1,
      code_snapshot_count: 1,
      environment_captured: true,
    })),
    capsule_builds: publicationCapsuleBuilds,
    effective_capability_level: publicationReproductionRuns.length
      ? "reproduced"
      : publicationRevision().capability_level,
    reproduction_runs: publicationReproductionRuns,
    reproduction_results: publicationReproductionResults,
  });

  (window as any).__TAURI__ = {
    core: {
      Channel,
      invoke: async (cmd: string, args: any) => {
        ((window as any).__skillInvokeLog ??= []).push({ cmd, args });
        const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
        const plain = (value: any): any => {
          if (value instanceof Map) return Object.fromEntries([...value].map(([k, v]) => [k, plain(v)]));
          if (Array.isArray(value)) return value.map(plain);
          if (value && typeof value === "object") return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, plain(v)]));
          return value;
        };
        switch (cmd) {
          case "start_window_move":
            return null;
          case "review_session": {
            const delay = Number((window as any).__reviewDelayMs ?? 0);
            if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
            return null;
          }
          case "get_research_graph":
            return researchGraph;
          case "get_publication_workspace":
            return publicationWorkspace();
          case "create_publication_workspace":
            return publicationWorkspace();
          case "save_publication_item": {
            const input = plain(arg("input") ?? {});
            publicationItems.push({
              id: `publication-item-${publicationItems.length + 1}`,
              revision_id: publicationRevisionId,
              parent_item_id: input.parentItemId ?? null,
              kind: String(input.kind ?? "claim"),
              title: String(input.title ?? "Untitled"),
              content: String(input.content ?? ""),
              ordinal: Number(input.ordinal ?? publicationItems.length),
            });
            return publicationWorkspace();
          }
          case "bind_publication_evidence": {
            const input = plain(arg("input") ?? {});
            const artifact = input.sourceKind === "artifact";
            const sourceKind = artifact ? "artifact_version" : String(input.sourceKind ?? "run");
            publicationBindings = [...publicationBindings, {
              id: `publication-binding-${publicationBindings.length + 1}`,
              revision_id: String(input.revisionId ?? publicationRevisionId),
              item_id: input.itemId ?? null,
              source_kind: sourceKind,
              source_id: artifact ? "artifact-version-v3" : String(input.sourceId ?? ""),
              purpose: String(input.purpose ?? ""),
              supported_claim_item_id: input.supportedClaimItemId ?? null,
              selection_state: String(input.selectionState ?? "selected"),
              review_state: "unreviewed",
              reproduction_state: "not_run",
              visibility: String(input.visibility ?? "public"),
              source_snapshot_json: "{}",
            }];
            return publicationWorkspace();
          }
          case "update_publication_evidence_binding": {
            const input = plain(arg("input") ?? {});
            publicationBindings = publicationBindings.map((binding) =>
              binding.id === input.bindingId
                ? {
                    ...binding,
                    selection_state: String(input.selectionState ?? binding.selection_state),
                    visibility: String(input.visibility ?? binding.visibility),
                  }
                : binding
            );
            return publicationWorkspace();
          }
          case "clone_publication_revision":
            publicationRevisionId = "publication-revision-2";
            publicationRevisionState = "draft";
            publicationBindings = publicationBindings.map((binding) => ({
              ...binding,
              revision_id: publicationRevisionId,
            }));
            return publicationWorkspace();
          case "save_publication_waiver":
            return publicationWorkspace();
          case "freeze_publication_revision":
            publicationRevisionState = "frozen";
            return {
              frozen: true,
              revision: publicationRevision(),
              readiness: publicationReadiness(),
            };
          case "build_publication_capsule": {
            const build = {
              id: `capsule-build-${publicationCapsuleBuilds.length + 1}`,
              revision_id: String(arg("revisionId") ?? publicationRevisionId),
              format: "zip",
              visibility: "public",
              status: "succeeded",
              output_path: "/exports/publication-capsule.zip",
              revision_manifest_sha256: "a".repeat(64),
              archive_sha256: "c".repeat(64),
              error: null,
              created_at: 1785480100,
              completed_at: 1785480101,
            };
            publicationCapsuleBuilds = [build, ...publicationCapsuleBuilds];
            return build;
          }
          case "verify_publication_revision": {
            const input = plain(arg("input") ?? {});
            const reproductionId = `reproduction-${publicationReproductionRuns.length + 1}`;
            publicationReproductionRuns = [{
              id: reproductionId,
              source_run_id: String(input.sourceRunId ?? "run-kinase-001"),
              status: "completed",
              capability_level: "reproduced",
              expected_environment_hash: "d".repeat(64),
              actual_environment_hash: "d".repeat(64),
              environment_matched: true,
              stdout_tail: "verification complete",
              stderr_tail: null,
              exit_code: 0,
              error: null,
              created_at: 1785480200,
              completed_at: 1785480201,
            }, ...publicationReproductionRuns];
            publicationReproductionResults = [{
              reproduction_run_id: reproductionId,
              output_id: "run-output-1",
              output_path: "results/figure2b.png",
              comparator_kind: "sha256",
              required: true,
              passed: true,
              report_json: "{}",
            }, ...publicationReproductionResults];
            return publicationWorkspace();
          }
          case "list_library_items":
            return libraryItems.map(librarySummary);
          case "list_session_library_items":
            return libraryItems
              .filter((item) => item.source_session_id === String(arg("sessionId") ?? ""))
              .map(({ base64: _base64, ...item }) => item);
          case "search_library_items": {
            const query = String(arg("query") ?? "").toLocaleLowerCase();
            const kind = arg("kind");
            return libraryItems
              .filter((item) => !kind || item.kind === kind)
              .filter((item) => [
                item.title,
                item.code,
                item.source_project_name,
                item.source_session_title,
              ].some((value) => String(value ?? "").toLocaleLowerCase().includes(query)))
              .map(librarySummary);
          }
          case "star_library_code": {
            const sessionId = String(arg("sessionId") ?? "");
            const language = String(arg("language") ?? "");
            const code = String(arg("code") ?? "");
            const existing = libraryItems.find((item) => item.kind === "code"
              && item.source_session_id === sessionId && item.language === language && item.code === code);
            if (existing) return existing;
            const item = {
              id: `library-${libraryItems.length + 1}`,
              kind: "code",
              title: code.split("\n").find((line) => line.trim())?.trim() ?? "Code",
              language,
              code,
              content_type: null,
              source_project_id: activeProjectId,
              source_project_name: activeProjectId === "other" ? "Other project" : project.name,
              source_session_id: sessionId,
              source_session_title: "Current analysis",
              source_path: null,
              created_at: Math.floor(Date.now() / 1000),
              base64: null,
            };
            libraryItems.unshift(item);
            return item;
          }
          case "star_library_text": {
            const sessionId = String(arg("sessionId") ?? "");
            const text = String(arg("text") ?? "");
            const existing = libraryItems.find((item) => item.kind === "text"
              && item.source_session_id === sessionId && item.code === text);
            if (existing) return existing;
            const item = {
              id: `library-${libraryItems.length + 1}`,
              kind: "text",
              title: text.split("\n").find((line) => line.trim())?.trim() ?? "Text",
              language: null,
              code: text,
              content_type: null,
              source_project_id: activeProjectId,
              source_project_name: activeProjectId === "other" ? "Other project" : project.name,
              source_session_id: sessionId,
              source_session_title: "Current analysis",
              source_path: null,
              created_at: Math.floor(Date.now() / 1000),
              base64: null,
            };
            libraryItems.unshift(item);
            return item;
          }
          case "star_library_figure": {
            const sessionId = String(arg("sessionId") ?? "");
            const path = String(arg("path") ?? "").replaceAll("\\", "/").replace(/^\.\//, "");
            const existing = libraryItems.find((item) => item.kind === "figure"
              && item.source_session_id === sessionId && item.source_path === path);
            if (existing) return existing;
            const item = {
              id: `library-${libraryItems.length + 1}`,
              kind: "figure",
              title: String(arg("name") ?? "Figure"),
              language: "python",
              code: "import matplotlib\nplt.savefig('volcano.png')",
              content_type: "image/png",
              source_project_id: activeProjectId,
              source_project_name: activeProjectId === "other" ? "Other project" : project.name,
              source_session_id: sessionId,
              source_session_title: "Current analysis",
              source_path: path,
              created_at: Math.floor(Date.now() / 1000),
              base64: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z0mAAAAAASUVORK5CYII=",
            };
            libraryItems.unshift(item);
            return item;
          }
          case "get_library_item": {
            const item = libraryItems.find((entry) => entry.id === arg("id"));
            if (!item) throw new Error("Library item not found");
            return item;
          }
          case "list_library_item_versions": {
            const item = libraryItems.find((entry) => entry.id === arg("id"));
            if (!item) return [];
            const original = {
              id: item.id,
              item_id: item.id,
              version_number: 1,
              parent_version_id: null,
              language: item.language,
              code: item.code,
              origin: "original",
              created_at: item.created_at,
            };
            return [original, ...(libraryVersions[item.id] ?? [])];
          }
          case "update_library_code": {
            const item = libraryItems.find((entry) => entry.id === arg("id"));
            if (!item) throw new Error("Library item not found");
            if (item.kind === "text") throw new Error("Text excerpts cannot be edited");
            const edits = (libraryVersions[item.id] ??= []);
            const head = edits[edits.length - 1];
            const version = {
              id: `library-version-${item.id}-${edits.length + 2}`,
              item_id: item.id,
              version_number: (head?.version_number ?? 1) + 1,
              parent_version_id: head?.id ?? item.id,
              language: arg("language") ?? head?.language ?? item.language,
              code: String(arg("code") ?? ""),
              origin: "edit",
              created_at: Math.floor(Date.now() / 1000),
            };
            edits.push(version);
            return version;
          }
          case "delete_library_item": {
            const before = libraryItems.length;
            libraryItems = libraryItems.filter((entry) => entry.id !== arg("id"));
            delete libraryVersions[String(arg("id"))];
            return libraryItems.length !== before;
          }
          case "list_demos":
            return demos;
          case "load_demo":
            return demo;
          case "copy_demo_to_project":
            return `copied-${String(arg("id") ?? "demo")}`;
          case "load_session_trajectory": {
            const frameId = String(arg("frameId") ?? "t1");
            const override = (window as any).__trajectorySnapshot;
            if (override) return { ...override, frame_id: frameId };
            return {
              frame_id: frameId,
              model: "deepseek-v4-pro",
              turns: [
                {
                  index: 1,
                  started_at: 1755000000000,
                  cells: [
                    { kind: "user", summary: "Analyze the ESR1 dataset", detail_input: null, detail_output: "Analyze the ESR1 dataset", ok: null, is_error: false, ts: 1755000000000, duration_ms: null, usage: null },
                    { kind: "assistant", summary: "I will inspect the counts matrix first.", detail_input: null, detail_output: "I will inspect the counts matrix first.", ok: null, is_error: false, ts: 1755000000500, duration_ms: 1200, usage: null },
                    { kind: "tool", summary: "python · df.describe()", detail_input: "{\"code\": \"df.describe()\"}", detail_output: "count  612.0\nmean  4.2", ok: true, is_error: false, ts: 1755000002000, duration_ms: 3400, usage: null },
                    { kind: "usage", summary: "", detail_input: null, detail_output: null, ok: null, is_error: false, ts: 1755000005600, duration_ms: null, usage: { round: 1, model: "deepseek-v4-pro", input_tokens: 12300, output_tokens: 1400, reasoning_tokens: 300, cached_input_tokens: 9225 } },
                  ],
                },
                {
                  index: 2,
                  started_at: 1755000060000,
                  cells: [
                    { kind: "user", summary: "Now plot the volcano", detail_input: null, detail_output: null, ok: null, is_error: false, ts: 1755000060000, duration_ms: null, usage: null },
                    { kind: "tool", summary: "python · sns.scatterplot()", detail_input: "{\"code\": \"sns.scatterplot()\"}", detail_output: "ValueError: missing column log2fc", ok: false, is_error: true, ts: 1755000060500, duration_ms: 800, usage: null },
                    { kind: "assistant", summary: "The plot failed; retrying with matplotlib.", detail_input: null, detail_output: "The plot failed; retrying with matplotlib.", ok: null, is_error: false, ts: 1755000061500, duration_ms: 2100, usage: null },
                    { kind: "usage", summary: "", detail_input: null, detail_output: null, ok: null, is_error: false, ts: 1755000064000, duration_ms: null, usage: { round: 2, model: "deepseek-v4-pro", input_tokens: 15000, output_tokens: 900, reasoning_tokens: 0, cached_input_tokens: 12000 } },
                  ],
                },
              ],
              stats: {
                turns: 2,
                steps: 4,
                llm_ms: 3300,
                tool_ms: 4200,
                input_tokens: 27300,
                output_tokens: 2300,
                cached_input_tokens: 21225,
                cache_hit_pct: 75.0,
                tokens_per_sec: 12.5,
              },
            };
          }
          case "load_session":
            if (mockExplorationFlow && String(arg("id") ?? "").startsWith("exploration-")) {
              activeMockFrame = String(arg("id"));
              return explorationTranscript(activeMockFrame);
            }
            if (mockBranchFlow) {
              const id = String(arg("id") ?? "");
              const session = mockSessions.find((row) => row.id === id);
              if (session) {
                const isBranch = Boolean(session.branched_from);
                return {
                  items: [
                    { role: "user", text: "Read the paper", tool_name: null, ok: null },
                    { role: "assistant", text: "Paper checkpoint", tool_name: null, ok: null },
                    ...(isBranch
                      ? [
                          { role: "user", text: `${session.title} task`, tool_name: null, ok: null },
                          { role: "assistant", text: `${session.title} result`, tool_name: null, ok: null },
                        ]
                      : [
                          { role: "user", text: "Main continued independently", tool_name: null, ok: null },
                          { role: "assistant", text: "Main current result", tool_name: null, ok: null },
                        ]),
                  ],
                  next_before_seq: null,
                  user_offset: 0,
                  branches: isBranch
                    ? []
                    : mockSessions
                        .filter((row) => row.branched_from === id)
                        .map((row) => ({
                          id: row.id,
                          title: String(row.title).replace(/^Branch: /, ""),
                          source_session_id: id,
                          checkpoint_user_index: 0,
                          checkpoint_kind: "after_response",
                          merged: Boolean(row.branch_state === "merged"),
                          merge_summary: row.branch_state === "merged" ? mockBranchMergedSummary : null,
                        })),
                  branch_state: session.branch_state ?? null,
                };
              }
            }
            if (quickActionSessions[String(arg("id") ?? "")]) {
              return {
                items: [{
                  role: "user",
                  text: quickActionSessions[String(arg("id"))],
                  tool_name: null,
                  ok: null,
                }],
                next_before_seq: null,
                user_offset: 0,
              };
            }
            if (mockPlanFlow) {
              return {
                items: [
                  { role: "user", text: "Prepare the regression plan", tool_name: null, ok: null },
                  // This is the load_session row produced from the persisted
                  // plan tool message — `wisp:plan` for ACP, the `propose_plan`
                  // result for built-in; LoadedItem::into_chat rebuilds both.
                  {
                    role: "plan",
                    text: JSON.stringify({
                      v: 1,
                      source: mockPlanFlow === "native" ? "native" : "acp",
                      entries: [
                        { content: "Inspect the existing behavior", status: "completed", priority: "medium" },
                        { content: "Wire the plan flow", status: "in_progress", priority: "high" },
                        { content: "Run regression checks", status: "pending", priority: "low" },
                      ],
                    }),
                    tool_name: null,
                    ok: null,
                  },
                ],
                next_before_seq: null,
                user_offset: 0,
              };
            }
            if (mockBrowserRestore && String(arg("id") ?? "") === "browser-restore-session") {
              return {
                items: [
                  { role: "user", text: "CLEC12A PubMed", tool_name: null, ok: null },
                  {
                    role: "tool",
                    text: "{\n  \"status\": \"disconnected\",\n  \"live_retrieval\": false\n}",
                    tool_name: "browser_setup",
                    ok: true,
                  },
                  {
                    role: "tool",
                    text: "{\n  \"status\": \"connected\",\n  \"live_retrieval\": true,\n  \"connected_tabs\": 1\n}",
                    tool_name: "browser_setup",
                    ok: true,
                  },
                  {
                    role: "tool",
                    text: "{\"tabs\":[{\"title\":\"PubMed CLEC12A\",\"url\":\"https://pubmed.ncbi.nlm.nih.gov\"}]}",
                    tool_name: "web_scan",
                    ok: true,
                  },
                  { role: "assistant", text: "PubMed currently lists live hits for CLEC12A.", tool_name: null, ok: null },
                ],
                next_before_seq: null,
                user_offset: 0,
                presentations: [{
                  presentation_id: "",
                  presentation_kind: "browser_disconnected",
                  payload: { code: "browser_extension_disconnected", live_retrieval: false },
                }],
              };
            }
            if (mockMcpAppSession) {
              return {
                items: [
                  { role: "user", text: "Open my saved workbench", tool_name: null, ok: null },
                  { role: "assistant", text: "The workbench is ready.", tool_name: null, ok: null },
                ],
                next_before_seq: null,
                user_offset: 0,
                presentations: [{
                  presentation_id: "saved-motif-workbench",
                  presentation_kind: "mcp_app",
                  payload: {
                    tool: { name: "motif_open_workbench", title: "Restored Motif workbench" },
                    arguments: { sequence: "ACGT" },
                    result: { content: [], structuredContent: { restored: true } },
                    resource: {
                      uri: "ui://motif/workbench.html",
                      text: `<!doctype html><html><body><div id="state">waiting</div><script>
                        addEventListener("message", (event) => {
                          const message = event.data || {};
                          if (message.id === 1 && message.result?.hostInfo?.name === "wisp-science") {
                            document.getElementById("state").textContent =
                              message.result?.hostCapabilities?.serverTools ? "restored-with-tools" : "restored";
                            parent.postMessage({ jsonrpc: "2.0", method: "ui/notifications/initialized", params: {} }, "*");
                          }
                        });
                        parent.postMessage({ jsonrpc: "2.0", id: 1, method: "ui/initialize", params: { protocolVersion: "2026-01-26" } }, "*");
                      <\/script></body></html>`,
                      _meta: {},
                    },
                  },
                }],
              };
            }
            if (query.get("mockSessionModels") === "1") {
              const id = String(arg("id") ?? "");
              return {
                items: [
                  { role: "user", text: `Question in ${id}`, tool_name: null, ok: null },
                  { role: "assistant", text: `Answer in ${id}`, tool_name: null, ok: null },
                ],
                next_before_seq: null,
                user_offset: 0,
              };
            }
            if (mockResourceSession) {
              return {
                items: [{
                  role: "assistant",
                  text: "[Open bound report](D:/ZZM/03.%20figures/report.md')\n\n[Open bound manuscript](/abs/path/D:/ZZM/paper/manuscript.docx)\n\n[Open bound references](references.bib)\n\n[Open bound Python script](analysis/scripts/random_walk_demo.py)\n\n[Open bound R script](analysis/plot.R)",
                  tool_name: null,
                  ok: null,
                  resources: [
                    {
                      id: "resource-link-markdown",
                      ordinal: 0,
                      originalReference: "D:/ZZM/03.%20figures/report.md'",
                      artifactId: "resource-artifact-markdown",
                      artifactVersionId: "resource-version-markdown",
                      displayName: "report.md",
                      kind: "markdown",
                      mimeType: "text/markdown",
                      status: "ready",
                      error: null,
                    },
                    {
                      id: "resource-link-docx",
                      ordinal: 1,
                      originalReference: "/abs/path/D:/ZZM/paper/manuscript.docx",
                      artifactId: "resource-artifact-docx",
                      artifactVersionId: "resource-version-docx",
                      displayName: "manuscript.docx",
                      kind: "docx",
                      mimeType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                      status: "ready",
                      error: null,
                    },
                    {
                      id: "resource-link-bib",
                      ordinal: 2,
                      originalReference: "references.bib",
                      artifactId: "resource-artifact-bib",
                      artifactVersionId: "resource-version-bib",
                      displayName: "references.bib",
                      kind: "text",
                      mimeType: "text/x-bibtex",
                      status: "ready",
                      error: null,
                    },
                    {
                      id: "resource-link-python",
                      ordinal: 3,
                      originalReference: "analysis/scripts/random_walk_demo.py",
                      artifactId: "resource-artifact-python",
                      artifactVersionId: "resource-version-python",
                      displayName: "random_walk_demo.py",
                      kind: "code",
                      mimeType: "text/x-python",
                      status: "ready",
                      error: null,
                    },
                    {
                      id: "resource-link-r",
                      ordinal: 4,
                      originalReference: "analysis/plot.R",
                      artifactId: "resource-artifact-r",
                      artifactVersionId: "resource-version-r",
                      displayName: "plot.R",
                      kind: "code",
                      mimeType: "text/x-r",
                      status: "ready",
                      error: null,
                    },
                  ],
                }],
                next_before_seq: null,
                user_offset: 0,
              };
            }
            if (mockLongSession) {
              const before = arg("beforeSeq");
              ((window as any).__transcriptPageCalls ??= []).push(before ?? null);
              const outline = before == null && mockLongPages === 0
                  ? Array.from({ length: 20 }, (_, index) => ({
                    user_index: index,
                    seq: 1 + index * 4,
                    sent_at: 1783478400 + index * 60,
                    response_at: 1783478430 + index * 60,
                    text: index === 0
                      ? "Oldest loaded question"
                      : index < 10
                        ? `Earlier transcript row ${index * 2}`
                        : index === 10
                          ? "Newest page first question"
                          : `Newest transcript row ${(index - 10) * 2}`,
                  }))
                : [];
              if (mockLongPages > 0) {
                const pageIndex = before == null ? 0 : Number(before);
                return {
                  items: Array.from({ length: mockLongRows }, (_, index) => ({
                    role: index % 2 === 0 ? "user" : "assistant",
                    text: `Window page ${pageIndex} row ${index} ${"x".repeat(mockLongRowBytes)}`,
                    tool_name: null,
                    ok: null,
                  })),
                  next_before_seq: pageIndex + 1 < mockLongPages ? pageIndex + 1 : null,
                  user_offset: Math.max(0, (mockLongPages - pageIndex - 1) * 10),
                };
              }
              if (before != null) {
                return {
                  items: Array.from({ length: 20 }, (_, index) => ({
                    role: index % 2 === 0 ? "user" : "assistant",
                    text: index === 0 ? "Oldest loaded question" : `Earlier transcript row ${index}`,
                    tool_name: null,
                    ok: null,
                  })),
                  next_before_seq: null,
                  user_offset: 0,
                };
              }
              return {
                items: Array.from({ length: 20 }, (_, index) => ({
                  role: index % 2 === 0 ? "user" : "assistant",
                  text: index === 0 ? "Newest page first question" : `Newest transcript row ${index}`,
                  tool_name: null,
                  ok: null,
                })),
                next_before_seq: 41,
                user_offset: 10,
                outline,
              };
            }
            return { items: [], next_before_seq: null, user_offset: 0 };
          case "list_sessions_page": {
            ((window as any).__projectSessionRefreshes ??= []).push(activeProjectId);
            const cursor = plain(arg("cursor"));
            const start = cursor ? mockSessions.findIndex((item) => item.id === cursor.id) + 1 : 0;
            const items = mockSessions.slice(start, start + 100);
            const hasMore = start + items.length < mockSessions.length;
            const last = items.at(-1);
            return {
              items,
              next_cursor: hasMore && last ? { id: last.id, ts: last.ts } : null,
              running_ids: mockSessions.filter((item) => item.running).map((item) => item.id),
            };
          }
          case "latest_used_session":
            return mockSessions.find((session) => session.has_user_turn !== false)?.id ?? null;
          case "list_project_explorations":
            return mockExplorations.filter((item) =>
              !mockExplorationRoundResolved || item.exploration.status !== "discarded").map((item) => ({
              ...item,
              exploration: { ...item.exploration },
            }));
          case "start_exploration": {
            const sourceFrameId = String(arg("sourceFrameId") ?? "");
            const sourceSession = mockSessions.find((session) => session.id === sourceFrameId);
            if (sourceSession?.branched_from || sourceSession?.branch_state) {
              throw new Error(
                "exploration_branch_unsupported: Conversation branches cannot start an exploration.",
              );
            }
            ((window as any).__startExplorationCalls ??= []).push({
              sourceFrameId,
              turnIndex: arg("turnIndex"),
              name: arg("name"),
            });
            const index = mockExplorations.length + 1;
            const id = `exploration-created-${index}`;
            const frameId = `exploration-frame-created-${index}`;
            const exploration = makeMockExploration(id, frameId, String(arg("name") ?? `Exploration ${index}`), 2100 + index);
            mockExplorations.push({
              exploration,
              source_frame_id: String(arg("sourceFrameId") ?? "exploration-mainline"),
              checkpoint_user_index: Number(arg("turnIndex") ?? 0),
              isolation_summary_json: '{"partial":false}',
            });
            activeMockFrame = frameId;
            return { ...exploration };
          }
          case "open_exploration": {
            const row = mockExplorations.find((item) => item.exploration.id === arg("explorationId"));
            if (!row) throw new Error("Exploration not found");
            activeMockFrame = row.exploration.frame_id;
            return { ...row.exploration };
          }
          case "preview_exploration_promotion":
            return mockExplorationPreview(String(arg("explorationId")));
          case "open_exploration_manual_resolution": {
            const preview = mockExplorationPreview(String(arg("explorationId")));
            if (!preview.eligibility.manualResolutionAvailable) {
              throw new Error("ExplorationNotPromotable: this exploration does not have a file-level manual resolution path");
            }
            return null;
          }
          case "promote_exploration": {
            const id = String(arg("explorationId"));
            const preview = mockExplorationPreview(id);
            if (!preview.eligibility.eligible) throw new Error("MainlineAdvanced: the mainline no longer matches this exploration checkpoint");
            const row = mockExplorations.find((item) => item.exploration.id === id)!;
            const adoptedFrame = row.source_frame_id;
            const mainline = mockSessions.find((item) => item.id === adoptedFrame);
            if (mainline) mainline.ts = 2200;
            mockExplorations = mockExplorations.filter((item) => item.source_frame_id !== adoptedFrame);
            activeMockFrame = adoptedFrame;
            mockExplorationRoundResolved = true;
            return { mainlineFrameId: adoptedFrame };
          }
          case "discard_exploration": {
            const row = mockExplorations.find((item) => item.exploration.id === arg("explorationId"));
            if (!row) throw new Error("Exploration not found");
            mockExplorations = mockExplorations.filter((item) => item.exploration.id !== row.exploration.id);
            return null;
          }
          case "abandon_exploration_round": {
            const sourceFrameId = String(arg("sourceFrameId"));
            mockExplorations = mockExplorations.filter((item) => item.source_frame_id !== sourceFrameId);
            mockExplorationRoundResolved = true;
            activeMockFrame = sourceFrameId;
            return null;
          }
          case "list_codex_sessions":
            return mockCodexSessions.map((item) => ({ ...item }));
          case "list_claude_sessions":
            return mockClaudeSessions.map((item) => ({ ...item }));
          case "preview_codex_session":
            return [
              { role: "user", text: "Fix the renderer crash\nIt fails after opening a second window." },
              { role: "assistant", text: "I will inspect the renderer lifecycle first." },
            ];
          case "preview_claude_session":
            return [
              { role: "user", text: `Review ${String(arg("path") ?? "this conversation")}` },
              { role: "assistant", text: "I will start with the relevant files." },
            ];
          case "import_codex_sessions": {
            if (nextSessionImportDelayMs > 0) {
              const delay = nextSessionImportDelayMs;
              nextSessionImportDelayMs = 0;
              await new Promise((resolve) => setTimeout(resolve, delay));
            }
            const paths = (plain(arg("paths")) ?? []) as string[];
            let imported = 0;
            const syncedPaths: string[] = [];
            for (const item of mockCodexSessions) {
              if (paths.includes(item.path) && item.state !== "imported") {
                item.state = "imported";
                imported += 1;
                syncedPaths.push(item.path);
                if (!mockFolders.some((folder) => folder.name.toLowerCase() === "codex")) {
                  mockFolders.push({ id: "codex-folder", name: "codex" });
                }
                if (!mockSessions.some((session) => session.id === `imported-${item.session_id}`)) {
                  mockSessions.push({
                    id: `imported-${item.session_id}`,
                    title: item.title,
                    ts: item.last_active_at,
                    running: false,
                    pinned: false,
                    folder_id: "codex-folder",
                  });
                }
              }
            }
            return { imported, updated: 0, skipped: paths.length - imported, failed: 0, synced_paths: syncedPaths };
          }
          case "import_claude_sessions": {
            if (nextSessionImportDelayMs > 0) {
              const delay = nextSessionImportDelayMs;
              nextSessionImportDelayMs = 0;
              await new Promise((resolve) => setTimeout(resolve, delay));
            }
            const paths = (plain(arg("paths")) ?? []) as string[];
            let imported = 0;
            const syncedPaths: string[] = [];
            for (const item of mockClaudeSessions) {
              if (paths.includes(item.path) && item.state !== "imported") {
                item.state = "imported";
                imported += 1;
                syncedPaths.push(item.path);
                if (!mockFolders.some((folder) => folder.name.toLowerCase() === "claude")) {
                  mockFolders.push({ id: "claude-folder", name: "claude" });
                }
                if (!mockSessions.some((session) => session.id === `imported-${item.session_id}`)) {
                  mockSessions.push({
                    id: `imported-${item.session_id}`,
                    title: item.title,
                    ts: item.last_active_at,
                    running: false,
                    pinned: false,
                    folder_id: "claude-folder",
                  });
                }
              }
            }
            return { imported, updated: 0, skipped: paths.length - imported, failed: 0, synced_paths: syncedPaths };
          }
          case "list_folders":
            ((window as any).__projectFolderRefreshes ??= []).push(activeProjectId);
            return mockFolders.map((folder) => ({ ...folder }));
          case "create_folder":
          case "rename_folder":
          case "delete_folder":
          case "move_session":
            return null;
          case "list_projects":
            return [
              { id: "default", name: projectNames.default ?? project.name, workspace_dir: project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0, sync_configured: syncedProjects.has("default"), last_synced_at: syncedProjects.has("default") ? Math.floor(Date.now() / 1000) : null },
              { id: "other", name: projectNames.other ?? "Other project", workspace_dir: "/mock/other", session_count: 1, updated_at: 1, running_count: 0, needs_you_count: 0, sync_configured: syncedProjects.has("other"), last_synced_at: syncedProjects.has("other") ? Math.floor(Date.now() / 1000) : null },
            ];
          case "list_recent_sessions":
            return [
              {
                id: "s-needs-you",
                project_id: "default",
                title: "帮我找一篇单细胞的文章",
                ts: 1,
                status: "needs_you",
              },
              {
                id: "s-complete",
                project_id: "default",
                title: "Enumerate MCP bio-tools databases",
                ts: 2,
                status: "complete",
              },
            ];
          case "pick_directory":
            return "/mock/root/new-project";
          case "pick_executable_file":
            return "/mock/picked/Rscript";
          case "open_project": {
            const openingProjectId = String(arg("id") ?? "default");
            const delay = nextProjectOpenDelayMs[openingProjectId] ?? 0;
            delete nextProjectOpenDelayMs[openingProjectId];
            if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
            if (failNextProjectOpenId === openingProjectId) {
              failNextProjectOpenId = null;
              throw new Error(`mock failed to open ${openingProjectId}`);
            }
            activeProjectId = openingProjectId;
            ((window as any).__projectOpenCompletions ??= []).push(activeProjectId);
            return { id: activeProjectId, name: projectNames[activeProjectId] ?? (activeProjectId === "other" ? "Other project" : project.name), workspace_dir: activeProjectId === "other" ? "/mock/other" : project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 };
          }
          case "create_project":
            activeProjectId = "default";
            return { id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 };
          case "import_project": {
            const delay = nextProjectTransferDelayMs.import ?? 0;
            delete nextProjectTransferDelayMs.import;
            if (delay > 0) await new Promise((resolve) => setTimeout(resolve, Math.min(delay, 40)));
            emit("project-transfer-progress", {
              direction: "import", stage: "extracting", completedFiles: 1, totalFiles: 2,
              projectId: "default", completedBytes: 512, totalBytes: 1024,
              currentPath: "workspace/data/example.tsv",
            });
            if (delay > 40) await new Promise((resolve) => setTimeout(resolve, delay - 40));
            return { id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 };
          }
          case "join_synced_project":
            return { id: "other", name: "Other project", workspace_dir: "/mock/other", session_count: 1, updated_at: 2, running_count: 0, needs_you_count: 0 };
          case "export_project": {
            const delay = nextProjectTransferDelayMs.export ?? 0;
            delete nextProjectTransferDelayMs.export;
            emit("project-transfer-progress", {
              direction: "export", stage: "writing", completedFiles: 1, totalFiles: 2,
              projectId: String(arg("id") ?? "default"), completedBytes: 512,
              totalBytes: 1024, currentPath: "data/example.tsv",
            });
            if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
            return "/mock/wisp-project.zip";
          }
          case "sync_project":
            if ((window as any).__failSyncConflict) {
              (window as any).__failSyncConflict = false;
              throw new Error("Sync conflict: this device and another device both changed the project. No data was overwritten.");
            }
            syncedProjects.add(String(arg("id") ?? "default"));
            return { status: "synced", direction: "push", revision: "revision-1", uploadedFiles: 1, downloadedFiles: 0, skippedPaths: [] };
          case "resolve_project_sync":
            return { status: "synced", direction: arg("strategy") === "remote" ? "pull" : "push", revision: "revision-2", uploadedFiles: 1, downloadedFiles: 1, skippedPaths: [] };
          case "project_sync_code":
            return "wisp-sync:mock-secret-code";
          case "delete_project":
            return null;
          case "open_project_window":
            return `proj-${arg("id")}`;
          case "get_bootstrap_status":
            return {
              skills_loaded: 12,
              python_ok: true,
              python_initializing: false,
              mcp_catalog: 8,
              uv_ok: true,
              node_ok: true,
              npm_ok: true,
              sci_ok: true,
              pixi_ok: true,
              app_version: "0.29.0",
              os: "windows",
              arch: "x86_64",
              startup: "total=120ms store=90ms window_ready=600000ms",
              workspace: project.root,
              errors: [],
            };
          case "get_appearance_prefs":
            return (window as any).__mockAppearancePrefs ?? {
              saved: false,
              theme: "system",
              light_palette: "paper",
              dark_palette: "charcoal",
              ui_font_size: 14,
              code_font_size: 12,
              ui_font_family: "",
              code_font_family: "",
              selection_popup_enabled: true,
              send_with_modifier: false,
              custom_css: "",
            };
          case "set_appearance_prefs": {
            const next = plain(arg("prefs") ?? args ?? {});
            (window as any).__mockAppearancePrefs = {
              saved: true,
              theme: String(next.theme ?? "system"),
              light_palette: String(next.light_palette ?? "paper"),
              dark_palette: String(next.dark_palette ?? "charcoal"),
              ui_font_size: Number(next.ui_font_size ?? 14),
              code_font_size: Number(next.code_font_size ?? 12),
              ui_font_family: String(next.ui_font_family ?? ""),
              code_font_family: String(next.code_font_family ?? ""),
              selection_popup_enabled: next.selection_popup_enabled !== false,
              send_with_modifier: Boolean(next.send_with_modifier),
              custom_css: String(next.custom_css ?? ""),
            };
            return (window as any).__mockAppearancePrefs;
          }
          case "get_settings":
            return {
              provider: "",
              api_url: "https://api.deepseek.com",
              model: "deepseek-v4-pro",
              has_api_key: true,
              locale: mockLocale,
              max_iter: 100,
              auto_compact: true,
              auto_continue: false,
              auto_continue_limit: 10,
              follow_up_questions: true,
              resume_last_session: true,
              max_tokens: 4096,
              reasoning_effort: "",
              supports_vision: true,
              sync_backend: "relay",
              sync_relay_url: mockSyncUnconfigured ? "" : "https://relay.example.test",
              sync_folder: "",
              sync_relay_token: "",
              has_sync_relay_token: !mockSyncUnconfigured,
              pet_enabled: mockPetEnabled,
              pet_directory: mockPetDirectory,
            };
          case "get_browser_url_filters":
            return mockBrowserUrlFilters;
          case "get_browser_auto_launch":
            return mockBrowserAutoLaunch;
          case "set_browser_auto_launch":
            mockBrowserAutoLaunch = Boolean(arg("enabled"));
            return mockBrowserAutoLaunch;
          case "get_browser_auto_close_tabs":
            return mockBrowserAutoCloseTabs;
          case "set_browser_auto_close_tabs":
            mockBrowserAutoCloseTabs = Boolean(arg("enabled"));
            return mockBrowserAutoCloseTabs;
          case "list_pending_browser_tab_cleanups":
            return mockPendingBrowserTabCleanups;
          case "confirm_browser_tab_cleanup": {
            const turnId = String(arg("turnId") ?? "");
            mockPendingBrowserTabCleanups = mockPendingBrowserTabCleanups.filter((row: any) => row.turn_id !== turnId);
            return (arg("tabs") ?? []).map((tab: any) => Number(tab.tab_id)).filter((id: number) => Number.isInteger(id));
          }
          case "dismiss_browser_tab_cleanup": {
            const turnId = String(arg("turnId") ?? "");
            mockPendingBrowserTabCleanups = mockPendingBrowserTabCleanups.filter((row: any) => row.turn_id !== turnId);
            return null;
          }
          case "set_browser_url_filters": {
            const next = plain(arg("filters") ?? {});
            mockBrowserUrlFilters = {
              block: Array.isArray(next.block) ? next.block.map((row: any) => ({
                host: String(row.host ?? "").trim().toLowerCase(),
                reason: String(row.reason ?? "").trim(),
              })).filter((row: { host: string }) => row.host) : [],
              prefer: Array.isArray(next.prefer) ? next.prefer.map((row: any) => ({
                host: String(row.host ?? "").trim().toLowerCase(),
                reason: String(row.reason ?? "").trim(),
              })).filter((row: { host: string }) => row.host) : [],
            };
            return mockBrowserUrlFilters;
          }
          case "get_context_usage_details":
            return {
              system_prompt: "You are wisp-science.\n\n## Environment\nWindows x86_64",
              tool_definitions: [
                { name: "read", description: "Read a file from disk." },
                { name: "write", description: "Write a file to disk." },
              ],
              rules: "## Built-in Rules\n\nVerify before completion.",
              skills: "## Skills Selection Guidelines\n\nUse use_skill before proceeding.",
              mcp_dynamic_tools: [
                { name: "search_mcp_tools", description: "Search configured MCP tools." },
              ],
              subagent_definitions: [
                { name: "explore", description: "Explore the project independently." },
              ],
            };
          case "get_token_usage":
            return {
              workspaces: [
                {
                  project_id: "default",
                  name: project.name,
                  workspace_dir: project.root,
                  updated_at: Math.floor(Date.now() / 1000),
                  session_count: 23,
                  input: 120000,
                  output: 30000,
                  reasoning: 8000,
                  cached: 90000,
                },
                {
                  project_id: "other",
                  name: "Other project",
                  workspace_dir: "/mock/other",
                  updated_at: Math.floor(Date.now() / 1000) - 3600,
                  session_count: 2,
                  input: 20000,
                  output: 5000,
                  reasoning: 1000,
                  cached: 12000,
                },
              ],
              days: Array.from({ length: 371 }, (_, index) => {
                const date = new Date(Date.UTC(2025, 7, 4 + index));
                return {
                  date: date.toISOString().slice(0, 10),
                  tokens: index % 9 === 0 ? (index + 1) * 75 : 0,
                  future: index > 366,
                };
              }),
              models: [
                { model: "deepseek-v4-pro", tokens: 120000 },
                { model: "opus-4.8", tokens: 30000 },
              ],
              tools: [
                { kind: "skill", name: "bear-support", calls: 12 },
                { kind: "mcp", name: "pubmed_search", calls: 8 },
                { kind: "skill", name: "bear-map", calls: 3 },
              ],
            };
          case "get_session_token_usage": {
            const projectId = String(arg("projectId") ?? "default");
            const total = projectId === "default" ? 23 : 2;
            const offset = Math.max(0, Number(arg("offset") ?? 0));
            const limit = Math.max(1, Number(arg("limit") ?? 20));
            const items = Array.from({ length: total }, (_, index) => ({
              id: `${projectId}-usage-${index + 1}`,
              title: `${projectId === "default" ? "Workspace" : "Other"} session ${index + 1}`,
              updated_at: Math.floor(Date.now() / 1000) - index * 60,
              input: 5000 + index,
              output: 1000 + index,
              reasoning: 200 + index,
              cached: 3000 + index,
            }));
            return { items: items.slice(offset, offset + limit), total };
          }
          case "get_pet":
            return {
              enabled: mockPetEnabled,
              directory: mockPetDirectory,
              error: null,
              asset: mockPetEnabled ? {
                id: "wispy",
                displayName: "Wispy",
                description: "A cheerful neon terminal spirit.",
                spriteVersionNumber: 2,
                spritesheetDataUrl: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z0mAAAAAASUVORK5CYII=",
                frameCounts: { idle: 7, "running-right": 8, "running-left": 8, waving: 4, jumping: 5, failed: 8, waiting: 6, running: 6, review: 6 },
              } : null,
            };
          case "get_pet_runtime_status":
            return {
              running: [],
              waiting: [],
              reviewing: [],
              activeRuns: (window as any).__mockPetActiveRuns ?? [],
            };
          case "set_pet_window_visible":
            (window as any).__petWindowVisible = Boolean(arg("visible"));
            return null;
          case "list_models":
            return mockModels;
          case "model_catalog_lookup": {
            return mockCatalogLookup(
              String(arg("provider") ?? ""),
              String(arg("apiUrl") ?? ""),
              String(arg("model") ?? ""),
            );
          }
          case "get_storage_usage":
            return {
              data_dir: "C:\\mock\\AppData\\wisp-science",
              projects: [
                { id: "default", name: project.name, path: project.root, bytes: 96 * 1024 * 1024 },
                { id: "other", name: "Other project", path: "/mock/other", bytes: 24 * 1024 * 1024 },
              ],
              entries: [
                { key: "database", bytes: 23 * 1024 * 1024 },
                { key: "python", bytes: 428 * 1024 * 1024 },
                { key: "plugins", bytes: 5632 * 1024 },
                { key: "workspace", bytes: 120 * 1024 * 1024 },
                { key: "other", bytes: 300 * 1024 },
              ],
              total_bytes: (23 + 428 + 120) * 1024 * 1024 + 5632 * 1024 + 300 * 1024,
            };
          case "get_session_model": {
            const sessionId = String(arg("sessionId") ?? "");
            return sessionModels[sessionId] ?? activeHttpModelId();
          }
          case "get_session_reasoning_effort": {
            const sessionId = String(arg("sessionId") ?? "");
            if (Object.prototype.hasOwnProperty.call(sessionReasoningEfforts, sessionId)) {
              return sessionReasoningEfforts[sessionId];
            }
            return null;
          }
          case "get_session_service_tier": {
            const sessionId = String(arg("sessionId") ?? "");
            if (Object.prototype.hasOwnProperty.call(sessionServiceTiers, sessionId)) {
              return sessionServiceTiers[sessionId];
            }
            return null;
          }
          case "list_acp_agents":
            return mockAcpAgents;
          case "get_dynamic_agent_options":
            return mockDynamicAgentOptions;
          case "plan_skill_portfolio":
            return {
              plan: {
                planner_model_id: String(plain(arg("request") ?? {}).model_id ?? "default"),
                planner_model_label: String(plain(arg("request") ?? {}).model_id) === "opus" ? "opus-4.8" : "deepseek-v4-pro",
                rationale: "Literature and analysis should run before evidence-grounded synthesis.",
                tasks: [
                  { id: "literature", rationale: "Find and verify published evidence.", skill_ids: ["literature-review"], depends_on: [] },
                  { id: "analysis", rationale: "Analyze the research question using the reproducible workflow.", skill_ids: ["analysis-workflow"], depends_on: [] },
                  { id: "synthesis", rationale: "Identify gaps only after both evidence streams finish.", skill_ids: [], depends_on: ["literature", "analysis"] },
                ],
              },
              proposal: {
                goal: "Design an evidence-grounded oncology study",
                context: "Design an oncology omics study",
                approval_policy: "review_all",
                tasks: [
                  { id: "literature", instruction: "Review the published evidence", depends_on: [], capabilities: ["literature_search"], skill_ids: ["literature-review"], specialist_id: null, output_schema: null, isolated: false, model_id: null, executor: null, budget: null },
                  { id: "analysis", instruction: "Plan a reproducible analysis", depends_on: [], capabilities: ["code_run"], skill_ids: ["analysis-workflow"], specialist_id: null, output_schema: null, isolated: false, model_id: null, executor: null, budget: null },
                  { id: "synthesis", instruction: "Synthesize the evidence and identify gaps", depends_on: ["literature", "analysis"], capabilities: ["reasoning"], skill_ids: [], specialist_id: null, output_schema: null, isolated: false, model_id: null, executor: null, budget: null },
                ],
              },
            };
          case "list_quick_actions":
            return mockQuickActions;
          case "list_workflow_templates":
            return mockWorkflowTemplates;
          case "save_quick_action": {
            const next = plain(arg("action"));
            if (!next?.id) next.id = `quick_action_${mockQuickActions.length}`;
            const existing = mockQuickActions.findIndex((item) => item.id === next.id);
            if (existing >= 0) mockQuickActions[existing] = next;
            else mockQuickActions.push(next);
            mockQuickActions.sort((left, right) => left.sort_order - right.sort_order);
            return mockQuickActions;
          }
          case "remove_quick_action":
            mockQuickActions = mockQuickActions.filter((item) => item.id !== arg("actionId"));
            return mockQuickActions;
          case "save_workflow_template": {
            const next = plain(arg("template"));
            if (!next?.id) next.id = `workflow_${mockWorkflowTemplates.length}`;
            next.builtin = false;
            const existing = mockWorkflowTemplates.findIndex((item) => item.id === next.id);
            if (existing >= 0) mockWorkflowTemplates[existing] = next;
            else mockWorkflowTemplates.push(next);
            return next;
          }
          case "remove_workflow_template":
            mockWorkflowTemplates = mockWorkflowTemplates.filter(
              (item) => item.id !== arg("templateId"),
            );
            return mockWorkflowTemplates;
          case "run_quick_action": {
            const action = mockQuickActions.find((item) => item.id === arg("actionId"));
            if (!action) throw new Error("Quick Action does not exist.");
            const input = plain(arg("input")) ?? {};
            const selection = String(input.selection ?? "").trim();
            if (!selection) throw new Error("Select some text before running this Quick Action.");
            const sessionId = `quick-action-${++mockAgentWorkflowCounter}`;
            const source = input.sourcePath ? ` from \`${String(input.sourcePath)}\`` : "";
            const displayMessage = `Run Quick Action “${action.name}” for the selected passage${source}:\n\n> ${selection}`;
            quickActionSessions[sessionId] = displayMessage;
            sessionDelegationEnabled[sessionId] = true;
            sessionModels[sessionId] = activeHttpModelId();
            lastDelegationSessionId = sessionId;
            const snapshot = dynamicWorkflowSnapshot({
              goal: "Review the literature evidence for a selected passage",
              context: selection,
              approval_policy: "auto_safe",
              tasks: [
                { id: "supporting_evidence", instruction: "Find supporting evidence", depends_on: [], capabilities: ["literature_search"], isolated: false },
                { id: "challenging_evidence", instruction: "Find challenging evidence", depends_on: [], capabilities: ["literature_search"], isolated: false },
                { id: "synthesize", instruction: "Synthesize the evidence", depends_on: ["supporting_evidence", "challenging_evidence"], capabilities: ["reasoning"], isolated: false },
              ],
            });
            snapshot.workflow.status = "approved";
            snapshot.workflow.requires_confirmation = false;
            snapshot.workflow.version += 1;
            snapshot.delegation_enabled = true;
            mockAgentWorkflows = [snapshot, ...mockAgentWorkflows];
            mockSessions.unshift({
              id: sessionId,
              title: action.name,
              ts: Date.now(),
              running: true,
            });
            void executeMockDynamicWorkflow(snapshot);
            return {
              action,
              session_id: sessionId,
              display_message: displayMessage,
              workflow: snapshot,
              started: true,
            };
          }
          case "list_agent_workflows": {
            const sessionId = String(arg("sessionId") ?? "");
            if (!sessionId) return [];
            const roots = new Set(mockAgentWorkflows
              .filter((item) => item.workflow.depth === 0 && item.workflow.frame_id === sessionId)
              .map((item) => item.workflow.id));
            return roots.size > 0
              ? mockAgentWorkflows.filter((item) => roots.has(item.workflow.root_workflow_id))
              : mockAgentWorkflows.filter((item) => item.workflow.frame_id === sessionId);
          }
          case "approve_agent_workflow": {
            const snapshot = mockAgentWorkflows.find((item) => item.workflow.id === arg("workflowId"));
            if (!snapshot) throw new Error("Agent workflow does not exist");
            if (!snapshot.delegation_enabled) throw new Error("Sub-Agent delegation is off for this conversation.");
            snapshot.workflow.status = "approved";
            snapshot.workflow.version += 1;
            if (snapshot.workflow.mode === "automatic") {
              void executeMockDynamicWorkflow(snapshot);
            }
            return snapshot;
          }
          case "run_agent_workflow": {
            const snapshot = mockAgentWorkflows.find((item) => item.workflow.id === arg("workflowId"));
            if (!snapshot) throw new Error("Agent workflow does not exist");
            if (!snapshot.delegation_enabled) throw new Error("Sub-Agent delegation is off for this conversation.");
            return executeMockDynamicWorkflow(snapshot);
          }
          case "cancel_agent_workflow": {
            const snapshot = mockAgentWorkflows.find((item) => item.workflow.id === arg("workflowId"));
            if (!snapshot) throw new Error("Agent workflow does not exist");
            snapshot.workflow.status = "cancelled";
            for (const task of snapshot.dynamic.tasks) {
              if (task.result?.status === "running") task.result = dynamicResult(task, "cancelled");
              else if (!task.result) task.result = dynamicResult(task, "blocked", { child_frame_id: null });
            }
            return null;
          }
          case "retry_agent_workflow": {
            const snapshot = mockAgentWorkflows.find((item) => item.workflow.id === arg("workflowId"));
            if (!snapshot) throw new Error("Agent workflow does not exist");
            if (!snapshot.delegation_enabled) throw new Error("Sub-Agent delegation is off for this conversation.");
            snapshot.workflow.status = "approved";
            snapshot.workflow.version += 1;
            const overrides = arg("budgetOverrides") ?? {};
            for (const task of snapshot.dynamic.tasks) {
              if (overrides[task.id]?.max_tokens) {
                task.budget.max_tokens = overrides[task.id].max_tokens;
              }
              if (task.result?.status !== "succeeded") task.result = null;
            }
            if (snapshot.workflow.mode === "automatic") {
              void executeMockDynamicWorkflow(snapshot);
            }
            return snapshot;
          }
          case "discard_agent_workflow":
            mockAgentWorkflows = mockAgentWorkflows.filter((item) => item.workflow.id !== arg("workflowId"));
            return null;
          case "get_agent_workflow_result": {
            const snapshot = mockAgentWorkflows.find((item) => item.workflow.id === arg("workflowId"));
            const task = snapshot?.dynamic?.tasks?.find((item: any) => item.stored_step_id === arg("stepId"));
            if (!snapshot || !task?.result?.full_result_available) throw new Error("Agent workflow result is not available");
            return {
              workflow_id: snapshot.workflow.id,
              step_id: task.stored_step_id,
              attempt: 1,
              status: task.result.status,
              response: {
                request_id: `request-${task.id}`,
                status: task.result.status,
                output: {
                  task_id: task.id,
                  summary: task.result.summary,
                  files_changed: [`reports/${task.id}.md`],
                  diff_summary: `Created the ${task.id} report.`,
                  artifacts: [{
                    name: `${task.id}.md`,
                    kind: "markdown",
                    content: `# ${task.id} result\n\nReadable result content for **${task.id}**.`,
                  }],
                  evidence: [`evidence-for-${task.id}`],
                  tests: ["Structure check passed"],
                  risks: ["Mock evidence only"],
                },
                artifact_ids: [`declared:${task.id}.md`],
                artifacts: [{
                  id: `declared:${task.id}.md`,
                  name: `${task.id}.md`,
                  kind: "markdown",
                  path: null,
                }],
                evidence: [{
                  kind: "agent",
                  summary: `evidence-for-${task.id}`,
                  reference: null,
                }],
                usage: { input_tokens: 900, output_tokens: 240, tool_calls: 3, cost_microunits: 19000 },
                agent_session_id: null,
                child_frame_id: `agent-child-${task.id}`,
                error: null,
              },
            };
          }
          case "get_acp_session_state": {
            const frameId = String(arg("frameId") ?? "");
            if (!acpBindings[frameId]) return null;
            return {
              availableModes: mockPlanFlow === "compat"
                ? [
                    { id: "default", name: "Default" },
                    { id: "agent", name: "Agent" },
                  ]
                : [
                    { id: "default", name: "Default" },
                    { id: "plan", name: "Plan" },
                  ],
            };
          }
          case "get_acp_session_agent":
            return acpBindings[String(arg("frameId") ?? "")] ?? null;
          case "save_acp_agent": {
            const profile = { ...(plain(arg("profile")) ?? {}) };
            if (!profile.id) profile.id = `acp-${mockAcpAgents.length + 1}`;
            const index = mockAcpAgents.findIndex((agent) => agent.id === profile.id);
            if (index >= 0) mockAcpAgents[index] = profile;
            else mockAcpAgents.push(profile);
            return mockAcpAgents;
          }
          case "remove_acp_agent":
            mockAcpAgents = mockAcpAgents.filter((agent) => agent.id !== arg("id"));
            return mockAcpAgents;
          case "test_acp_agent":
            if (query.get("mockAcpTerminalAuth") === "1") {
              return {
                protocolVersion: 1,
                implementation: { name: "fake-claude-acp", title: "Claude Agent", version: "0.69.0" },
                capabilities: {},
                authMethods: [{
                  id: "claude-ai-login",
                  name: "Claude Subscription",
                  description: "Use Claude subscription",
                  type: "terminal",
                }],
              };
            }
            return {
              protocolVersion: 1,
              implementation: { name: "fake-acp", title: "Fake ACP", version: "1.0" },
              capabilities: { loadSession: true, sessionCapabilities: { configOptions: true } },
              authMethods: [{ id: "browser", name: "Sign in", description: "Authenticate in browser" }],
            };
          case "authenticate_acp_agent":
            if (query.get("mockAcpTerminalAuth") === "1") {
              const profile = mockAcpAgents.find((agent) => agent.id === arg("id"));
              return {
                id: `terminal-mock-${++terminalCounter}`,
                projectId: activeProjectId,
                contextId: `acp-auth:${String(arg("id") ?? "agent")}`,
                title: `${String(profile?.label ?? "ACP Agent")} — Claude Subscription`,
                kind: "acp-auth",
                displayCwd: "/mock/root",
                processId: 1234,
                running: true,
              };
            }
            return null;
          case "set_acp_session_config": {
            const configId = String(arg("configId") ?? "");
            const currentValue = plain(arg("value"))?.value;
            return [
              { id: "model", name: "Model", type: "select", currentValue: configId === "model" ? currentValue : "smart", options: [{ value: "fast", name: "Fast" }, { value: "smart", name: "Smart" }] },
              { id: "fast_mode", name: "Fast Mode", type: "boolean", currentValue: configId === "fast_mode" ? Boolean(currentValue) : false },
            ];
          }
          case "set_acp_session_mode":
            return String(arg("modeId") ?? "");
          case "respond_acp_permission":
            setTimeout(() => {
              const requestId = String(arg("requestId"));
              const frameId = acpPermissionFrames[requestId] ?? "";
              emit("permission-resolved", { frameId, requestId });
              emit("agent", { kind: "Done", frame_id: frameId, stop_reason: "end_turn" });
              delete acpPermissionFrames[requestId];
            }, 0);
            return null;
          case "respond_ask_user":
            setTimeout(() => {
              const requestId = String(arg("requestId"));
              const frameId = askUserFrames[requestId] ?? "";
              emit("ask-user-resolved", { frameId, requestId, expired: false });
              delete askUserFrames[requestId];
            }, 0);
            return null;
          case "credential_status":
            return Object.entries(mockCredentials);
          case "list_custom_credentials":
            return mockCustomCredentials.map((credential) => ({ ...credential }));
          case "channels_status":
            return { ...mockChannels, device: { ...mockChannels.device } };
          case "set_feishu_channel":
            mockChannels.feishu_enabled = Boolean(arg("enabled"));
            mockChannels.feishu_international = Boolean(arg("international"));
            mockChannels.feishu_app_id = String(arg("appId") ?? "");
            if (String(arg("appSecret") ?? "").trim()) {
              mockChannels.feishu_has_secret = true;
            }
            mockChannels.feishu_bound = Boolean(mockChannels.feishu_app_id && mockChannels.feishu_has_secret);
            mockChannels.feishu_state = mockChannels.feishu_enabled ? "running" : "stopped";
            return null;
          case "feishu_bind_start":
            mockFeishuPollCount = 0;
            return {
              flow_id: "mock-feishu-flow",
              qr_image: "data:image/svg+xml;base64," + btoa('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 21"><rect width="21" height="21" fill="white"/><path d="M1 1h6v6H1zm2 2v2h2V3zM14 1h6v6h-6zm2 2v2h2V3zM1 14h6v6H1zm2 2v2h2v-2zM9 2h2v2H9zm2 3h2v2h-2zM8 8h3v3H8zm5 0h2v2h-2zm3 1h4v2h-4zM9 13h2v2H9zm3-2h2v4h-2zm3 2h2v2h-2zm3 0h2v4h-2zm-9 4h3v3H9zm5-1h3v2h-3zm1 3h5v1h-5z" fill="black"/></svg>'),
              expires_in_seconds: 600,
            };
          case "feishu_bind_poll":
            mockFeishuPollCount += 1;
            if (mockFeishuPollCount === 1) {
              return { state: "pending", retry_after_ms: 500, app_id: "" };
            }
            mockChannels.feishu_bound = true;
            mockChannels.feishu_has_secret = true;
            mockChannels.feishu_app_id = "cli_scan_created";
            mockChannels.feishu_owner_open_id = "ou_scan_owner";
            mockChannels.feishu_pending_owner_open_id = "";
            mockChannels.feishu_international = Boolean(arg("international") ?? mockChannels.feishu_international);
            return { state: "confirmed", retry_after_ms: 0, app_id: mockChannels.feishu_app_id };
          case "feishu_bind_cancel":
            return null;
          case "feishu_unbind":
            mockChannels.feishu_bound = false;
            mockChannels.feishu_enabled = false;
            mockChannels.feishu_has_secret = false;
            mockChannels.feishu_app_id = "";
            mockChannels.feishu_owner_open_id = "";
            mockChannels.feishu_pending_owner_open_id = "";
            mockChannels.feishu_state = "stopped";
            return null;
          case "set_feishu_owner":
            mockChannels.feishu_owner_open_id = String(arg("openId") ?? "").trim();
            mockChannels.feishu_pending_owner_open_id = "";
            return null;
          case "confirm_feishu_pending_owner":
            mockChannels.feishu_owner_open_id = mockChannels.feishu_pending_owner_open_id;
            mockChannels.feishu_pending_owner_open_id = "";
            return null;
          case "reject_feishu_pending_owner":
            mockChannels.feishu_pending_owner_open_id = "";
            return null;
          case "set_weixin_channel":
            mockChannels.weixin_enabled = Boolean(arg("enabled"));
            mockChannels.weixin_state = mockChannels.weixin_enabled ? "running" : "stopped";
            return null;
          case "weixin_bind_start":
            return { qrcode: "mock-qr", qr_image: "data:image/svg+xml;base64," + btoa('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 21 21"><rect width="21" height="21" fill="white"/><path d="M1 1h6v6H1zm2 2v2h2V3zM14 1h6v6h-6zm2 2v2h2V3zM1 14h6v6H1zm2 2v2h2v-2zM9 2h2v2H9zm2 3h2v2h-2zM8 8h3v3H8zm5 0h2v2h-2zm3 1h4v2h-4zM9 13h2v2H9zm3-2h2v4h-2zm3 2h2v2h-2zm3 0h2v4h-2zm-9 4h3v3H9zm5-1h3v2h-3zm1 3h5v1h-5z" fill="black"/></svg>') };
          case "weixin_bind_poll":
            mockChannels.weixin_bound = true;
            return "confirmed";
          case "weixin_unbind":
            mockChannels.weixin_bound = false;
            mockChannels.weixin_enabled = false;
            mockChannels.weixin_state = "stopped";
            return null;
          case "set_device_bridge": {
            const enabled = Boolean(arg("enabled"));
            const bindIpv4 = String(arg("bindIpv4") ?? "");
            const port = Number(arg("port") ?? 0);
            mockChannels.device.enabled = enabled;
            mockChannels.device.mode = String(arg("mode") ?? "lan");
            mockChannels.device.bindIpv4 = bindIpv4;
            mockChannels.device.port = port;
            mockChannels.device.detail = "";
            if (!enabled) {
              mockChannels.device.state = "stopped";
              mockChannels.device.url = null;
              mockChannels.device.hasToken = false;
              mockDeviceToken = "";
              return { ...mockChannels.device };
            }
            if (!mockDeviceToken) {
              mockDeviceTokenSequence += 1;
              mockDeviceToken = `mock-sticks3-token-${mockDeviceTokenSequence}`;
              mockChannels.device.hasToken = true;
            }
            if (query.get("mockDeviceBridgeError") === "1") {
              mockChannels.device.state = "error";
              mockChannels.device.url = null;
              mockChannels.device.detail = `Cannot listen on ${bindIpv4}:${port}: address already in use`;
              throw new Error(mockChannels.device.detail);
            }
            mockChannels.device.state = "listening";
            mockChannels.device.url = `http://${bindIpv4}:${port}`;
            return { ...mockChannels.device };
          }
          case "rotate_device_bridge_token":
            mockDeviceTokenSequence += 1;
            mockDeviceToken = `mock-sticks3-token-${mockDeviceTokenSequence}`;
            mockChannels.device.hasToken = true;
            return mockDeviceToken;
          case "get_device_bridge_token":
            if (!mockDeviceToken) throw new Error("No Device Bridge token exists. Generate one first.");
            return mockDeviceToken;
          case "revoke_device_bridge_token":
            mockDeviceToken = "";
            mockChannels.device.hasToken = false;
            return null;
          case "list_ssh_hosts":
            return [{
              alias: "gpu-server",
              user: "researcher",
              port: 22,
              identity_file: null,
              notes: "Mock GPU host",
            }];
          case "remove_ssh_host":
            return [];
          case "list_execution_contexts":
            return executionContexts;
          case "list_session_execution_context_ids": {
            const sessionId = String(arg("sessionId") ?? arg("session_id") ?? "");
            return [...(sessionExecutionContexts[sessionId] ?? [])];
          }
          case "set_session_execution_context_enabled": {
            const sessionId = String(arg("sessionId") ?? arg("session_id") ?? "");
            const contextId = String(arg("contextId") ?? arg("context_id") ?? "");
            const context = executionContexts.find((item) => item.id === contextId);
            if (!sessionId || !context || context.kind === "local") {
              throw new Error("Execution context not found");
            }
            const selected = new Set(sessionExecutionContexts[sessionId] ?? []);
            if (Boolean(arg("enabled"))) selected.add(contextId);
            else selected.delete(contextId);
            sessionExecutionContexts[sessionId] = [...selected].sort();
            return [...sessionExecutionContexts[sessionId]];
          }
          case "list_remote_files": {
            const contextId = String(arg("contextId") ?? arg("context_id") ?? "");
            return remoteStagingEntries.filter(
              (entry) => entry.context_id === contextId && !entry.removed_at
            );
          }
          case "remove_remote_files": {
            const contextId = String(arg("contextId") ?? arg("context_id") ?? "");
            const ids = (arg("ids") ?? []) as string[];
            const force = Boolean(arg("force"));
            for (const id of ids) {
              const entry = remoteStagingEntries.find(
                (item) => item.id === id && item.context_id === contextId && !item.removed_at
              );
              if (!entry) throw new Error(`remote file entry ${id} is not ledgered`);
              if (entry.state === "active" && !force) {
                throw new Error(`${entry.remote_path} is still referenced by a run`);
              }
              entry.removed_at = Math.floor(Date.now() / 1000);
            }
            return remoteStagingEntries.filter(
              (entry) => entry.context_id === contextId && !entry.removed_at
            );
          }
          case "context_disposal_report": {
            const contextId = String(arg("contextId") ?? arg("context_id") ?? "");
            return {
              context_id: contextId,
              external_references: Number((window as any).__mockExternalRefs ?? 0),
              staged_files: remoteStagingEntries.filter(
                (entry) => entry.context_id === contextId && !entry.removed_at
              ).length,
            };
          }
          case "get_context_storage_prefs": {
            const contextId = String(arg("contextId") ?? arg("context_id") ?? "");
            const stored = contextStoragePrefs[contextId];
            if (stored) return { ...stored, context_id: contextId, confirmed: true };
            const context = executionContexts.find((item) => item.id === contextId);
            const label = (context?.label ?? contextId).toLowerCase().replace(/[^a-z0-9]+/g, "-");
            return {
              context_id: contextId,
              remote_data_root: "~/wisp/demo-project/data",
              remote_workdir_root: ".wisp-science/runs",
              local_results_dir: `remote/${label}`,
              confirmed: false,
            };
          }
          case "set_context_storage_prefs": {
            const contextId = String(arg("contextId") ?? arg("context_id") ?? "");
            const prefs = {
              remote_data_root: String(arg("remoteDataRoot") ?? arg("remote_data_root") ?? ""),
              remote_workdir_root: String(
                arg("remoteWorkdirRoot") ?? arg("remote_workdir_root") ?? ""
              ),
              local_results_dir: String(arg("localResultsDir") ?? arg("local_results_dir") ?? ""),
            };
            if (!prefs.remote_data_root || !prefs.remote_workdir_root || !prefs.local_results_dir) {
              throw new Error("storage locations are required");
            }
            contextStoragePrefs[contextId] = prefs;
            return { ...prefs, context_id: contextId, confirmed: true };
          }
          case "get_default_execution_context":
            return defaultExecutionContext;
          case "set_default_execution_context": {
            const contextId = arg("contextId") ?? arg("context_id");
            if (contextId === null || contextId === undefined || String(contextId).trim() === "") {
              defaultExecutionContext = null;
              return null;
            }
            const context = executionContexts.find((item) => item.id === String(contextId));
            if (!context || context.kind === "local") {
              throw new Error("Execution context not found");
            }
            defaultExecutionContext = String(contextId);
            return defaultExecutionContext;
          }
          case "probe_execution_context": {
            const delay = nextProbeDelayMs;
            nextProbeDelayMs = 0;
            if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
            return executionContexts.find((context) =>
              context.id === String(arg("contextId") ?? arg("context_id"))
            ) ?? null;
          }
          case "update_execution_context_interpreters": {
            const context = executionContexts.find((item) =>
              item.id === String(arg("contextId") ?? arg("context_id"))
            );
            if (!context) throw new Error("Execution context not found");
            const config = JSON.parse(context.config_json || "{}");
            delete config.python_path;
            delete config.rscript_path;
            const python = String(arg("pythonExecutable") ?? arg("python_executable") ?? "").trim();
            const rscript = String(arg("rscriptExecutable") ?? arg("rscript_executable") ?? "").trim();
            if (python) config.python_executable = python;
            else delete config.python_executable;
            if (rscript) config.rscript_executable = rscript;
            else delete config.rscript_executable;
            context.config_json = JSON.stringify(config);
            return context;
          }
          case "list_runtimes":
            return runtimeInfos;
          case "execute_runtime": {
            // Echo the routing back as console text so a test can assert which
            // runtime the code was sent to, the way the real worker would.
            // Plotting code additionally reports a captured PNG snapshot.
            const code = String(arg("code") ?? "");
            // 1x1 transparent PNG, the smallest valid plot snapshot.
            const tinyPng =
              "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
            const plots = code.includes("plot(") || code.includes("plt.") ? [tinyPng] : [];
            if (code.includes("stop(")) return { text: `[error] ${code}`, plots: [] };
            return { text: `[${arg("language")} @ ${arg("contextId")}] ${code}`, plots };
          }
          case "execute_runtime_script": {
            const scriptPath = String(arg("scriptPath") ?? arg("script_path") ?? "");
            return {
              text: `[runtime script] path=${scriptPath} sha256=abc runtime_id=rt generation=1\n[${arg("language")} @ ${arg("contextId")}] ${scriptPath}`,
              plots: [],
            };
          }
          case "inspect_runtime":
            return {
              objects: [
                {
                  name: "counts",
                  typeName: "DataFrame",
                  summary: "12000000 × 48",
                  sizeBytes: 4 * 1024 * 1024 * 1024,
                },
                {
                  name: "model",
                  typeName: "RandomForestClassifier",
                  summary: "",
                  sizeBytes: null,
                },
              ],
              totalCount: 2,
            };
          case "start_runtime": {
            const contextId = String(arg("contextId") ?? arg("context_id"));
            const language = String(arg("language"));
            const info = {
              runtimeId: `runtime-${language}-${Date.now()}`,
              generation: 1,
              key: { projectId: activeProjectId, contextId, language },
              status: "ready",
              interpreter: language === "r" ? "/opt/R/bin/Rscript" : "/opt/python/bin/python",
              version: language === "r" ? "4.4.1" : "3.11.9",
              processId: 3301,
              startedAtMs: Date.now(),
              lastActivityAtMs: Date.now(),
              residentMemoryBytes: null,
              lastError: null,
            };
            runtimeInfos = runtimeInfos.filter((item) => !(
              item.key.projectId === activeProjectId
              && item.key.contextId === contextId
              && item.key.language === language
            ));
            runtimeInfos.push(info);
            return info;
          }
          case "stop_runtime": {
            const info = runtimeInfos.find((item) =>
              item.key.projectId === String(arg("projectId") ?? arg("project_id"))
              && item.key.contextId === String(arg("contextId") ?? arg("context_id"))
              && item.key.language === String(arg("language"))
            );
            if (info) {
              info.status = "dead";
              info.lastActivityAtMs = Date.now();
              info.processId = null;
            }
            return info ?? null;
          }
          case "restart_runtime": {
            const info = runtimeInfos.find((item) =>
              item.key.projectId === String(arg("projectId") ?? arg("project_id"))
              && item.key.contextId === String(arg("contextId") ?? arg("context_id"))
              && item.key.language === String(arg("language"))
            );
            if (info) {
              info.runtimeId = `runtime-restarted-${Date.now()}`;
              info.generation += 1;
              info.status = "ready";
              info.processId = 4401;
              info.lastActivityAtMs = Date.now();
              info.lastError = null;
            }
            return info ?? null;
          }
          case "import_wsl_contexts":
            return [
              ...executionContexts,
              {
                id: "wsl:Ubuntu-24.04",
                kind: "wsl",
                label: "Ubuntu-24.04",
                config_json: "{\"distro\":\"Ubuntu-24.04\"}",
                capabilities_json: "{}",
                last_probe_at: null,
                last_probe_status: null,
                last_probe_error: null,
                created_at: 1783478400,
                updated_at: 1783478400,
              },
            ];
          case "open_terminal": {
            const contextId = String(arg("contextId") ?? arg("context_id") ?? "local");
            return {
              id: `terminal-mock-${++terminalCounter}`,
              projectId: activeProjectId,
              contextId,
              title: `${contextId} — Terminal`,
              kind: contextId.startsWith("ssh:") ? "ssh" : "local",
              displayCwd: "/mock/root",
              processId: 1234,
              running: true,
            };
          }
          case "attach_terminal": {
            setTimeout(() => arg("onEvent")?.onmessage?.({
              event: "output",
              data: { base64: btoa("terminal ready\r\n") },
            }), 0);
            return {
              id: String(arg("sessionId") ?? "terminal-mock"),
              projectId: activeProjectId,
              contextId: "ssh:gpu-server",
              title: "ssh:gpu-server — Terminal",
              kind: "ssh",
              displayCwd: "/mock/root",
              processId: 1234,
              running: true,
            };
          }
          case "write_terminal":
          case "resize_terminal":
          case "close_terminal":
            return null;
          case "list_runs":
            return runs.map(runSummary);
          case "get_run_detail": {
            const run = runs.find((item) => item.id === String(arg("runId") ?? ""));
            if (!run) throw new Error("Run not found");
            return run;
          }
          case "list_run_workspace_files": {
            const runId = String(arg("runId") ?? "");
            const path = String(arg("path") ?? "");
            const filter = String(arg("nameFilter") ?? "").toLowerCase();
            const limit = Number(arg("limit") ?? 200);
            const offset = Number(arg("offset") ?? 0);
            const levels = runWorkspaceFiles[runId] ?? {};
            let rows = (levels[path] ?? []).filter((entry: any) =>
              !filter || entry.path.split("/").pop().toLowerCase().includes(filter)
            );
            const page = rows.slice(offset, offset + limit);
            return { entries: page, truncated: rows.length > offset + limit };
          }
          case "download_run_files":
            return [];
          case "delete_run_files": {
            const runId = String(arg("runId") ?? "");
            const paths = (arg("paths") ?? []) as string[];
            const levels = runWorkspaceFiles[runId] ?? {};
            for (const level of Object.keys(levels)) {
              levels[level] = levels[level].filter(
                (entry: any) => !paths.some((p) => entry.path === p || entry.path.startsWith(`${p}/`))
              );
            }
            for (const p of paths) delete levels[p];
            return null;
          }
          case "should_prompt_run_review": {
            const runId = String(arg("runId") ?? "");
            const run = runs.find((item) => item.id === runId);
            if (!run) throw new Error("Run not found");
            if (
              run.kind !== "ssh_direct" ||
              run.status !== "succeeded" ||
              run.cleaned_at ||
              runReviewDismissed.has(runId)
            ) {
              return false;
            }
            const specs = JSON.parse(String(run.output_specs_json ?? "[]"));
            if (Array.isArray(specs) && specs.length > 0) return !run.harvested_at;
            return ((runWorkspaceFiles[runId] ?? {})[""] ?? []).length > 0;
          }
          case "dismiss_run_review": {
            runReviewDismissed.add(String(arg("runId") ?? ""));
            return null;
          }
          case "cleanup_run_workspace": {
            const run = runs.find((item) => item.id === String(arg("runId") ?? ""));
            if (!run) throw new Error("Run not found");
            if (["submitted", "running", "cancelling"].includes(run.status)) {
              throw new Error("Run is still active");
            }
            if (!run.remote_handle_json) throw new Error("Run has no server workspace to clean");
            run.cleaned_at = run.cleaned_at ?? Math.floor(Date.now() / 1000);
            run.cleanup_error = null;
            return run;
          }
          case "harvest_run": {
            const run = runs.find((item) => item.id === String(arg("runId") ?? ""));
            if (!run) throw new Error("Run not found");
            run.harvested_at = run.harvested_at ?? Math.floor(Date.now() / 1000);
            return run;
          }
          case "get_method_search_run":
            return mockMethodSearchDetails();
          case "start_method_search": {
            const run = runs.find((item) => item.id === String(arg("runId") ?? ""));
            if (!run || run.kind !== "method_search" || run.status !== "draft") {
              throw new Error("Method-search Run could not start");
            }
            run.status = "submitted";
            run.started_at = Math.floor(Date.now() / 1000);
            run.progress_json = JSON.stringify({
              ...JSON.parse(run.progress_json),
              phase: "search",
            });
            return mockMethodSearchDetails();
          }
          case "pause_method_search": {
            const run = runs.find((item) => item.id === String(arg("runId") ?? ""));
            if (!run || !["submitted", "running"].includes(run.status)) {
              throw new Error("Method-search Run is not running");
            }
            run.status = "paused";
            return mockMethodSearchDetails();
          }
          case "resume_method_search": {
            const run = runs.find((item) => item.id === String(arg("runId") ?? ""));
            if (!run || run.status !== "paused") {
              throw new Error("Method-search Run is not paused");
            }
            run.status = "submitted";
            return mockMethodSearchDetails();
          }
          case "cancel_method_search": {
            const run = runs.find((item) => item.id === String(arg("runId") ?? ""));
            if (!run || run.kind !== "method_search") {
              throw new Error("Method-search Run does not exist");
            }
            run.status = "cancelled";
            run.ended_at = Math.floor(Date.now() / 1000);
            return mockMethodSearchDetails();
          }
          case "cancel_run": {
            const run = runs.find((r) => r.id === (arg("runId") ?? arg("run_id")));
            if (run) {
              run.status = "cancelled";
              run.ended_at = Math.floor(Date.now() / 1000);
            }
            if (run && monitorRunFrameId) {
              const frameId = monitorRunFrameId;
              setTimeout(() => {
                emit("agent", { kind: "ToolResult", frame_id: frameId, name: "monitor_run", ok: true, content: JSON.stringify(run) });
                emit("agent", { kind: "Done", frame_id: frameId, stop_reason: "end_turn" });
                resolveMonitorRun?.(frameId);
                resolveMonitorRun = null;
                monitorRunFrameId = null;
              }, 0);
            }
            return run ?? null;
          }
          case "save_model": {
            const profile = plain(arg("profile") ?? {});
            // Mirror the backend: catalog-known models clamp context/output
            // to their documented ceilings.
            const catalogEntry = mockCatalogLookup(
              String(profile.provider ?? ""),
              String(profile.api_url ?? ""),
              String(profile.model ?? ""),
            );
            if (catalogEntry) {
              if (typeof profile.context_window === "number" && profile.context_window > 0) {
                profile.context_window = Math.min(profile.context_window, catalogEntry.context_window);
              }
              if (typeof profile.max_tokens === "number" && profile.max_tokens > 0) {
                profile.max_tokens = Math.min(profile.max_tokens, catalogEntry.max_tokens);
              }
            }
            const useForVision = Boolean(arg("useForVision") ?? profile.use_for_vision);
            const useForImageGeneration = Boolean(
              arg("useForImageGeneration") ?? profile.use_for_image_generation,
            );
            const useForVideoGeneration = Boolean(
              arg("useForVideoGeneration") ?? profile.use_for_video_generation,
            );
            // Mirror the backend: an empty id creates a fresh profile.
            if (!profile.id) {
              let n = 1;
              while (mockModels.some((m) => m.id === `m${n}`)) n += 1;
              profile.id = `m${n}`;
              if (!profile.label) profile.label = profile.model;
              mockModels = [...mockModels, profile];
            }
            mockModels = mockModels.map((m) => m.id === profile.id ? {
              ...m,
              ...profile,
              use_for_vision: useForVision,
              use_for_image_generation: useForImageGeneration,
              use_for_video_generation: useForVideoGeneration,
            } : {
              ...m,
              use_for_vision: useForVision ? false : m.use_for_vision,
              use_for_image_generation: useForImageGeneration
                ? false
                : m.use_for_image_generation,
              use_for_video_generation: useForVideoGeneration
                ? false
                : m.use_for_video_generation,
            });
            return mockModels;
          }
          case "remove_model": {
            const id = arg("id") ?? "";
            mockModels = mockModels.filter((m) => m.id !== id);
            return mockModels;
          }
          case "set_active_model": {
            const id = arg("id") ?? "";
            const sessionId = String(arg("sessionId") ?? "");
            if (sessionId) {
              sessionModels[sessionId] = id;
            } else {
              mockModels = mockModels.map((m) => ({ ...m, active: m.id === id }));
            }
            return mockModels;
          }
          case "set_session_reasoning_effort": {
            const sessionId = String(arg("sessionId") ?? "");
            const effort = String(arg("effort") ?? "");
            // Empty effort clears the override; the session inherits the
            // bound profile again (mirrors src-tauri models.rs).
            if (effort === "") {
              delete sessionReasoningEfforts[sessionId];
            } else {
              sessionReasoningEfforts[sessionId] = effort;
            }
            return null;
          }
          case "set_session_service_tier": {
            const sessionId = String(arg("sessionId") ?? "");
            const tier = arg("serviceTier");
            if (tier === null || typeof tier === "undefined") {
              delete sessionServiceTiers[sessionId];
            } else {
              sessionServiceTiers[sessionId] = String(tier);
            }
            return null;
          }
          case "get_project_info":
            ((window as any).__projectInfoReads ??= []).push(activeProjectId);
            return activeProjectId === "other"
              ? { ...project, id: "other", name: "Other project", root: "/mock/other" }
              : project;
          case "generate_follow_up_questions":
            return [
              "Review the records that need manual correction",
              "Expand the search for underrepresented species",
              "Generate a literature landscape visualization",
            ];
          case "get_project_run_retention":
            return projectRunRetention;
          case "set_project_run_retention": {
            projectRunRetention = {
              run_retention_days: (arg("runRetentionDays") ?? null) as number | null,
              failed_run_retention_days:
                (arg("failedRunRetentionDays") ?? null) as number | null,
              orphan_file_retention_days:
                (arg("orphanFileRetentionDays") ?? null) as number | null,
            };
            return projectRunRetention;
          }
          case "get_project_settings": {
            const settingsId = String(arg("id") ?? activeProjectId ?? "default");
            return {
              id: settingsId,
              name: projectNames[settingsId] ?? (settingsId === "other" ? "Other project" : project.name),
              description: projectDescriptions[settingsId] ?? "",
              agent_context: projectAgentContexts[settingsId] ?? (settingsId === "default" ? projectAgentContext : ""),
            };
          }
          case "update_project": {
            const settingsId = String(arg("id") ?? activeProjectId ?? "default");
            const nextName = String(arg("name") ?? projectNames[settingsId] ?? project.name);
            if (nextName.trim()) {
              projectNames[settingsId] = nextName.trim();
              if (settingsId === "default" || settingsId === activeProjectId) {
                project.name = nextName.trim();
              }
            }
            const nextDescription = String(arg("description") ?? "");
            projectDescriptions[settingsId] = nextDescription;
            const nextContext = String(arg("agentContext") ?? arg("agent_context") ?? "");
            projectAgentContexts[settingsId] = nextContext;
            if (settingsId === "default" || settingsId === activeProjectId) {
              projectAgentContext = nextContext;
            }
            return null;
          }
          case "get_onboarding_state":
            return mockOnboarding ? { show: true, has_api_key: false } : { show: false, has_api_key: true };
          case "get_capabilities":
            return {
              skills,
              mcp_servers: ["mcp_bio", "mcp_chem"],
              memory_files: memoryFilesFor(activeProjectId),
              project,
              skill_counts: { bundled: 2, project: 1 },
              mcp_counts: { bundled: 2, project: 1 },
            };
          case "list_skills":
            return [
              ...skills,
              ...plugins.filter((plugin) => plugin.enabled).map((plugin) => ({
                name: "motif-for-claude-science",
                description: "Open the Motif molecular-biology workbench",
                tags: [],
                scope: "plugin",
                enabled: true,
                builtin: true,
                managed: true,
                managed_by: plugin.display_name,
                dir: "/plugins/motif/skills/hypothesis-review",
              })),
            ];
          case "reload_skills": {
            if (query.get("mockSkillReload") === "1" && !skills.some((skill) => skill.name === "fresh-project-skill")) {
              skills.push({
                name: "fresh-project-skill",
                description: "Newly copied project skill",
                tags: ["fresh"],
                scope: "project",
                enabled: true,
                builtin: false,
                dir: "/mock/project/.wisp/skills/fresh-project-skill",
              });
            }
            return [
              ...skills,
              ...plugins.filter((plugin) => plugin.enabled).map((plugin) => ({
                name: "motif-for-claude-science",
                description: "Open the Motif molecular-biology workbench",
                tags: [],
                scope: "plugin",
                enabled: true,
                builtin: true,
                managed: true,
                managed_by: plugin.display_name,
                dir: "/plugins/motif/skills/hypothesis-review",
              })),
            ];
          }
          case "pick_skill_source":
            return query.get("mockSkillImport") === "1"
              ? "/downloads/paper-narrative.zip"
              : null;
          case "install_skill":
            return "paper-narrative";
          case "remove_skill": {
            const name = String(arg("name") ?? "");
            skills = skills.filter((skill) => skill.name !== name || skill.builtin);
            return null;
          }
          case "list_plugins":
            return plugins;
          case "pick_plugin_source":
            return query.get("mockPluginImport") === "1"
              ? "/downloads/motif-update.zip"
              : null;
          case "install_plugin":
          case "install_plugin_url":
            return plugins[0] ?? null;
          case "set_plugin_enabled": {
            const pluginId = String(arg("pluginId") ?? "");
            const version = String(arg("version") ?? "");
            const enabled = Boolean(arg("enabled"));
            plugins = plugins.map((plugin) =>
              plugin.id === pluginId && plugin.version === version
                ? { ...plugin, enabled }
                : plugin,
            );
            return null;
          }
          case "remove_plugin": {
            const pluginId = String(arg("pluginId") ?? "");
            const version = String(arg("version") ?? "");
            plugins = plugins.filter((plugin) => plugin.id !== pluginId || plugin.version !== version);
            return null;
          }
          case "list_mcp_connections":
            return { connections: mockMcpConnections };
          case "list_connectors":
            return {
              scope: "ask",
              connectors: [
                {
                  key: "biomart",
                  name: "BioMart",
                  kind: "bundled",
                  enabled: true,
                  skip_approvals: false,
                  transport: "",
                  subtitle: "",
                  auth: "",
                  tools: [{ name: "biomart_query", mode: "allow", description: "" }],
                },
                ...mockMcpConnections.map((connection) => ({
                  key: connection.id,
                  name: connection.name,
                  kind: "custom",
                  enabled: connection.enabled,
                  skip_approvals: false,
                  transport: String(connection.transport?.kind ?? ""),
                  subtitle: connection.transport?.kind === "stdio"
                    ? String(connection.transport?.command ?? "")
                    : String(connection.transport?.url ?? ""),
                  auth: String(connection.transport?.auth ?? "none"),
                  tools: [],
                })),
              ],
            };
          case "list_approval_grants":
            return mockApprovalGrants;
          case "revoke_approval_grant": {
            const scope = String(arg("scope") ?? "");
            const kind = String(arg("kind") ?? "");
            const target = String(arg("target") ?? "");
            mockApprovalGrants = mockApprovalGrants.filter(
              (row) => row.scope !== scope || row.kind !== kind || row.target !== target,
            );
            return null;
          }
          case "revoke_all_approval_grants":
            mockApprovalGrants = [];
            return null;
          case "test_mcp_connection":
            return mockMcpTools;
          case "test_oauth_mcp_connection":
            if (mockOAuthPending) {
              await new Promise<void>((resolve) => {
                resolveMockOAuth = resolve;
              });
            }
            return mockMcpTools;
          case "set_mcp_connection_enabled": {
            const id = arg("id") ?? "";
            const enabled = Boolean(arg("enabled"));
            mockMcpConnections = mockMcpConnections.map((c) => c.id === id ? { ...c, enabled } : c);
            return null;
          }
          case "delete_mcp_connection": {
            const id = arg("id") ?? "";
            mockMcpConnections = mockMcpConnections.filter((c) => c.id !== id);
            return null;
          }
          case "add_mcp_connection":
          case "update_mcp_connection":
          case "set_connector_enabled":
          case "set_tool_approval":
          case "set_approval_scope":
          case "set_connector_skip_approvals":
            return null;
          case "authorize_http_connection": {
            const connection = plain(arg("conn") ?? {});
            mockMcpConnections = [
              ...mockMcpConnections.filter((item) => item.id !== connection.id),
              connection,
            ];
            return null;
          }
          case "set_credential": {
            const id = String(arg("id") ?? "");
            mockCredentials[id] = String(arg("value") ?? "").trim().length > 0;
            mockCustomCredentials = mockCustomCredentials.map((credential) =>
              credential.id === id
                ? { ...credential, present: mockCredentials[id] }
                : credential,
            );
            return null;
          }
          case "add_custom_credential": {
            const credential = {
              id: `custom-${nextCustomCredential++}`,
              name: String(arg("name") ?? "").trim(),
              envVar: String(arg("envVar") ?? "").trim(),
              present: String(arg("value") ?? "").trim().length > 0,
            };
            mockCustomCredentials.push(credential);
            mockCredentials[credential.id] = credential.present;
            return { ...credential };
          }
          case "remove_custom_credential": {
            const id = String(arg("id") ?? "");
            mockCustomCredentials = mockCustomCredentials.filter((credential) => credential.id !== id);
            delete mockCredentials[id];
            return null;
          }
          case "set_skill_tags": {
            const name = arg("name") ?? "";
            const tags = Array.isArray(arg("tags")) ? arg("tags") : [];
            skills = skills.map((s) => s.name === name ? { ...s, tags } : s);
            return null;
          }
          case "set_skill_enabled": {
            const name = arg("name") ?? "";
            const enabled = Boolean(arg("enabled"));
            skills = skills.map((s) => s.name === name ? { ...s, enabled } : s);
            return null;
          }
          case "set_skills_enabled": {
            const names = new Set(Array.isArray(arg("names")) ? arg("names") : []);
            const enabled = Boolean(arg("enabled"));
            skills = skills.map((s) => names.has(s.name) ? { ...s, enabled } : s);
            return null;
          }
          case "list_dir": {
            const cwd = String(arg("path") ?? ".").replaceAll("\\", "/").replace(/^\.\//, "").replace(/\/$/, "") || ".";
            return workspaceEntries
              .filter((entry) => {
                const split = entry.path.lastIndexOf("/");
                const parent = split < 0 ? "." : entry.path.slice(0, split);
                return parent === cwd;
              })
              .map((entry) => ({
                name: entry.path.slice(entry.path.lastIndexOf("/") + 1),
                is_dir: entry.is_dir,
                size: entry.size,
                modified_unix_millis: entry.modified_unix_millis,
              }))
              .sort((a, b) => Number(b.is_dir) - Number(a.is_dir) || a.name.localeCompare(b.name));
          }
          case "create_file": {
            const path = String(arg("path") ?? "");
            if (workspaceEntries.some((entry) => entry.path === path)) throw new Error(`workspace entry '${path}' already exists`);
            workspaceEntries.push({ path, is_dir: false, size: 0, modified_unix_millis: Date.now() });
            return null;
          }
          case "create_directory": {
            const path = String(arg("path") ?? "");
            if (workspaceEntries.some((entry) => entry.path === path)) throw new Error(`workspace entry '${path}' already exists`);
            workspaceEntries.push({ path, is_dir: true, size: 0, modified_unix_millis: Date.now() });
            return null;
          }
          case "rename_entry": {
            const path = String(arg("path") ?? "");
            const newPath = String(arg("newPath") ?? "");
            workspaceEntries = workspaceEntries.map((entry) => entry.path === path || entry.path.startsWith(`${path}/`)
              ? { ...entry, path: `${newPath}${entry.path.slice(path.length)}` }
              : entry);
            return null;
          }
          case "delete_entry": {
            const path = String(arg("path") ?? "");
            workspaceEntries = workspaceEntries.filter((entry) => entry.path !== path && !entry.path.startsWith(`${path}/`));
            return null;
          }
          case "upload_to_context": {
            const dest = String(arg("destinationDir") ?? "/home/research");
            const paths = Array.isArray(arg("sourcePaths")) ? arg("sourcePaths") : ["/mock/local/counts.csv"];
            const name = String(paths[0] ?? "counts.csv").split(/[\\/]/).pop() || "counts.csv";
            return [{
              sourcePath: String(paths[0] ?? "/mock/local/counts.csv"),
              destinationPath: `${dest.replace(/\/$/, "")}/${name}`,
              runId: "run-upload-1",
              status: "running",
            }];
          }
          case "list_remote_dir": {
            const path = String(arg("path") ?? "~");
            if (path === "/home/research/projects") {
              return {
                path,
                entries: [
                  { name: "rna-seq", is_dir: true, size: 0, modified_unix_millis: FILE_NOW - 86_400_000 },
                  { name: "README.md", is_dir: false, size: 512, modified_unix_millis: FILE_NOW - 2 * 86_400_000 },
                ],
              };
            }
            return {
              path: "/home/research",
                entries: [
                { name: "projects", is_dir: true, size: 0, modified_unix_millis: FILE_NOW - 86_400_000 },
                { name: "notes.txt", is_dir: false, size: 128, modified_unix_millis: FILE_NOW - 90_000 },
              ],
            };
          }
          case "search_files": {
            const q = String(arg("query") ?? "").toLowerCase();
            const all = [
              { path: "data/report.csv", name: "report.csv", is_dir: false, size: 4096 },
              { path: "counts.csv", name: "counts.csv", is_dir: false, size: 128 },
            ];
            return all.filter((h) => h.name.toLowerCase().includes(q));
          }
          case "search_artifacts": {
            const q = String(arg("query") ?? "").toLowerCase();
            return q ? artifacts.filter((a) => a.name.toLowerCase().includes(q)) : artifacts;
          }
          case "search_sessions": {
            const q = String(arg("query") ?? "").toLowerCase();
            const requestedProject = arg("projectId");
            const preferredProject = arg("preferredProjectId");
            const limit = Math.max(1, Math.min(100, Number(arg("limit") ?? 12)));
            if (requestedProject != null) {
              return mockSessions
                .filter((session) => [session.title, session.body]
                  .some((value) => String(value ?? "").toLowerCase().includes(q)))
                .slice(0, limit)
                .map((session) => ({
                  id: session.id,
                  project_id: String(requestedProject),
                  project_name: project.name,
                  title: session.title,
                  ts: session.ts,
                  activity_at: session.ts,
                  status: session.running ? "running" : "complete",
                }));
            }
            const rows = query.get("mockManySessions") === "1"
              ? mockSessions.map((session) => ({
                  id: session.id,
                  project_id: "default",
                  project_name: project.name,
                  title: session.title,
                  body: session.body ?? "",
                  ts: session.ts,
                  activity_at: session.ts,
                  status: session.running ? "running" : "complete",
                }))
              : [
                  { id: "s-current", project_id: "default", project_name: "wisp-science", title: "Current analysis", body: "The counts table is discussed in this transcript.", ts: 1, activity_at: 3, status: "complete" },
                  { id: "s-old", project_id: "default", project_name: "wisp-science", title: "Older structure run", body: "", ts: 1, activity_at: 2, status: "complete" },
                  { id: "s-other", project_id: "other", project_name: "Other project", title: "Cross-project counts", body: "", ts: 1, activity_at: 1, status: "needs_you" },
                  { id: "s-complete", project_id: "default", project_name: "wisp-science", title: "Enumerate MCP bio-tools databases", body: "", ts: 1, activity_at: 1, status: "complete" },
                ];
            return rows
              .filter((session) => !q
                || session.title.toLowerCase().includes(q)
                || session.body.toLowerCase().includes(q))
              .sort((left, right) => {
                const projectRank = Number(left.project_id !== preferredProject)
                  - Number(right.project_id !== preferredProject);
                if (projectRank) return projectRank;
                const titleRank = Number(!left.title.toLowerCase().includes(q))
                  - Number(!right.title.toLowerCase().includes(q));
                return titleRank || right.activity_at - left.activity_at;
              })
              .slice(0, limit)
              .map(({ body: _body, ...session }) => session);
          }
          case "save_file": {
            const path = String(arg("path") ?? "");
            savedWorkspaceFiles.set(path, String(arg("content") ?? ""));
            return null;
          }
          case "read_file": {
            const path = String(arg("path") ?? "report.csv");
            const saved = savedWorkspaceFiles.get(path);
            if (saved !== undefined) {
              return { path, mime: "text/plain", text: saved, base64: null };
            }
            if (path.toLowerCase().endsWith(".pdb")) {
              return { path, mime: "chemical/x-pdb", text: "ATOM      1  CA  ALA A   1      11.104  13.207   9.132  1.00 20.00           C\nEND\n", base64: null };
            }
            if (path.toLowerCase().endsWith(".fasta")) {
              return { path, mime: "text/plain", text: ">seq1\nMKTIIALSYIFCLVFADYKDDDDK\n>seq2\nMKTIIALSYIFCLVFADYKDDDDK\n", base64: null };
            }
            if (path.toLowerCase().endsWith(".r")) {
              // Multi-line on purpose: #307 collapsed a script's newlines into one
              // paragraph, which a single-line fixture cannot catch.
              return { path, mime: "text/x-r", text: workspaceR, base64: null };
            }
            if (path.toLowerCase().endsWith(".py")) {
              return { path, mime: "text/x-python", text: 'import scanpy as sc\nadata = sc.read("counts.h5ad")\n', base64: null };
            }
            if (path.toLowerCase().endsWith(".toml")) {
              return { path, mime: "application/octet-stream", text: '[project]\nname = "x"\n', base64: null };
            }
            if (path.toLowerCase().endsWith(".ipynb")) {
              const text = JSON.stringify({
                metadata: { kernelspec: { language: "python" } },
                cells: [
                  { cell_type: "markdown", source: ["## Saved notebook output\n"] },
                  {
                    cell_type: "code",
                    source: ["display(result)\n"],
                    outputs: [
                      {
                        output_type: "display_data",
                        data: {
                          "text/html": '<style>.saved-table{color:green}</style><table id="saved-table" class="saved-table"><tr><td>safe HTML result</td></tr></table><img id="external-image" src="https://example.invalid/pixel.png" onerror="parent.__notebookPwned=true"><script>parent.__notebookPwned=true</script>',
                        },
                      },
                      {
                        output_type: "display_data",
                        data: {
                          "image/svg+xml": '<svg xmlns="http://www.w3.org/2000/svg" width="80" height="30"><script>parent.__notebookPwned=true</script><rect width="80" height="30" fill="teal"/><text x="8" y="20">SVG result</text></svg>',
                        },
                      },
                      {
                        output_type: "execute_result",
                        data: { "text/latex": "\\frac{a}{b}" },
                      },
                      {
                        output_type: "display_data",
                        data: { "image/png": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z0mAAAAAASUVORK5CYII=" },
                      },
                    ],
                  },
                ],
              });
              return { path, mime: "application/x-ipynb+json", text, base64: null };
            }
            if (path.toLowerCase().endsWith(".unknown")) {
              return { path, mime: "application/octet-stream", text: null, base64: "AA==" };
            }
            if (path.toLowerCase().endsWith(".rtf")) {
              return { path, mime: "application/rtf", text: "# Experimental protocol\n\nCentrifuge at **12000 g**.", base64: null };
            }
            if (path.toLowerCase().includes(".pdf")) {
              return { path, mime: "application/pdf", text: null, base64: pdfBase64 };
            }
            if (path.toLowerCase().includes(".docx")) {
              return { path, mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document", text: null, base64: docxBase64 };
            }
            if (path.toLowerCase().includes(".png")) {
              return { path, mime: "image/png", text: null, base64: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z0mAAAAAASUVORK5CYII=" };
            }
            if (path.toLowerCase().endsWith(".mp4")) {
              return { path, mime: "video/mp4", text: null, base64: mp4Base64 };
            }
            if (path.toLowerCase().includes("figure_legend")) {
              return { path, mime: "text/markdown", text: "# Figure legend\n\nSee [the paper](https://example.com/paper) and [the plot](results/new.png).\n", base64: null };
            }
            if (path.toLowerCase().endsWith(".md")) {
              return { path, mime: "text/markdown", text: "# Draft manuscript\n\nOriginal body paragraph.\n", base64: null };
            }
            if (path.toLowerCase().includes(".json")) {
              return { path, mime: "application/json", text: '{"model":{"name":"wisp","enabled":true}}', base64: null };
            }
            if (path.toLowerCase().includes(".html")) {
              return { path, mime: "text/html", text: '<style>#mode::after{content:"Desktop"}@media(max-width:900px){#mode::after{content:"Mobile"}}</style><div id="mode"></div>', base64: null };
            }
            return { path, mime: "text/csv", text: "a,b\n1,2", base64: null };
          }
          case "read_file_bytes": {
            const path = String(arg("path") ?? "").toLowerCase();
            if (path.includes(".pdf")) return base64Bytes(pdfBase64);
            if (path.includes(".docx")) return base64Bytes(docxBase64);
            if (path.includes(".xlsx") && xlsxBase64) return base64Bytes(xlsxBase64);
            if (path.includes(".pptx") && pptxBase64) return base64Bytes(pptxBase64);
            // Chat media (generated-image/video cards, artifact thumbnails)
            // fetches bytes through the blob-object-URL pipeline.
            if (/\.(png|jpe?g|gif|webp|svg)$/.test(path)) {
              return base64Bytes("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z0mAAAAAASUVORK5CYII=");
            }
            if (path.includes(".mp4")) return base64Bytes(mp4Base64);
            throw new Error("Binary fixture not found");
          }
          case "read_artifact":
            if (arg("id") === "art-html") {
              return { path: "artifact:art-html", mime: "text/html", text: '<style>#mode::after{content:"Desktop"}@media(max-width:900px){#mode::after{content:"Mobile"}}</style><div id="mode"></div>', base64: null };
            }
            if (arg("id") === "art-markdown") {
              return { path: "artifact:art-markdown", mime: "text/markdown", text: "# Differential expression report\n\nRendered Markdown body.", base64: null };
            }
            return { path: `artifact:${arg("id")}`, mime: "text/csv", text: "a,b\n1,2", base64: null };
          case "read_artifact_version":
            if (arg("versionId") === "resource-version-markdown") {
              return {
                path: "artifact-version:resource-version-markdown",
                mime: "text/markdown",
                text: `# Bound report\n\n${Array.from({ length: 120 }, (_, index) => `Scrollable row ${index + 1}`).join("\n\n")}`,
                base64: null,
              };
            }
            if (arg("versionId") === "resource-version-docx") {
              return {
                path: "artifact-version:resource-version-docx",
                mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                text: null,
                base64: docxBase64,
              };
            }
            if (arg("versionId") === "resource-version-bib") {
              return {
                path: "artifact-version:resource-version-bib",
                mime: "text/x-bibtex",
                text: "@article{wisp,\n  title = {Wisp Science}\n}",
                base64: null,
              };
            }
            if (arg("versionId") === "resource-version-python") {
              return {
                path: "artifact-version:resource-version-python",
                mime: "text/x-python",
                text: "SEED = 42\nprint('random walk')\n",
                base64: null,
              };
            }
            if (arg("versionId") === "resource-version-r") {
              return {
                path: "artifact-version:resource-version-r",
                mime: "text/x-r",
                text: 'in_dir <- "data"\nplot(1:3)\n',
                base64: null,
              };
            }
            throw new Error("Artifact version not found");
          case "read_artifact_version_bytes":
            if (arg("versionId") === "resource-version-docx") {
              return base64Bytes(docxBase64);
            }
            throw new Error("Artifact version bytes not found");
          case "missing_files": {
            const paths = Array.isArray(arg("paths")) ? arg("paths") : [];
            return paths.filter((p) => String(p).includes("/.pdf") || String(p).includes("\\.pdf"));
          }
          case "append_review_note": {
            const src = String(arg("sourcePath") ?? "");
            const stem = (src.split(/[\\/]/).pop() ?? "notes").replace(/\.[^.]+$/, "") || "notes";
            return `reviews/${stem}.md`;
          }
          case "export_session":
            return "/mock/export.zip";
          case "export_session_trajectory":
            if ((window as any).__trajectoryExportCancel) return null;
            if ((window as any).__trajectoryExportError) {
              throw new Error(String((window as any).__trajectoryExportError));
            }
            return "/mock/wisp-trajectory.html";
          case "save_share_image":
            return "/mock/wisp-share.png";
          case "save_share_html":
            return "/mock/wisp-share.html";
          case "generate_share_social_copy": {
            const platform = String(arg("platform") ?? "twitter");
            const messages = Array.isArray(arg("messages")) ? arg("messages") : [];
            if (!messages.length) {
              throw new Error("Select at least one message.");
            }
            return {
              platform,
              highlights: [
                {
                  title: "Clean peak",
                  why: "The 530 nm assignment is unambiguous.",
                  messageIndexes: [1, 2],
                },
              ],
              variants: [
                {
                  title: `${platform} hook`,
                  body: `Paste-ready ${platform} copy about the clean 530 nm peak.`,
                  hashtags: ["#wisp", "#spectrum"],
                },
                {
                  title: `${platform} alt`,
                  body: `Another ${platform} angle: the spectrum came back clean.`,
                  hashtags: ["#lab"],
                },
                {
                  title: `${platform} short`,
                  body: `530 nm. Clean. Ready to share on ${platform}.`,
                  hashtags: [],
                },
              ],
            };
          }
          case "import_session_archive":
            return {
              frame_id: "imported-frame",
              status: "imported",
              message_count: 3,
              artifact_count: 0,
              missing_artifacts: [],
            };
          case "get_artifact_provenance":
            return {
              code: "import matplotlib\nplt.savefig('volcano.png')",
              language: "python",
              output: "saved volcano.png",
              exit_status: "ok",
              inputs: [{ path: "DE_results.csv", produced_here: false }],
              env: { name: "kernel", packages: [{ name: "matplotlib", version: "3.8.0" }] },
            };
          case "upload_file":
            return {
              id: "art-upload-1",
              name: arg("filename") ?? "upload.csv",
              kind: "text/csv",
              path: `uploads/${arg("filename") ?? "upload.csv"}`,
              ts: 1,
            };
          case "register_artifact": {
            const path = String(arg("path") ?? "");
            const name = path.split(/[\\/]/).pop() || "file";
            const artifact = {
              id: `art-registered-${artifacts.length + 1}`,
              name,
              kind: name.toLowerCase().endsWith(".csv") ? "text/csv" : "application/octet-stream",
              path: `/mock/root/${path}`,
              ts: Math.floor(Date.now() / 1000),
              project_id: activeProjectId,
              project_name: activeProjectId === "other" ? "Other project" : project.name,
              session_id: "s-current",
              session_title: "Current analysis",
              size_bytes: null,
              origin: "artifact",
            };
            artifacts.unshift(artifact);
            return artifact;
          }
          case "set_settings": {
            const next = plain(arg("settings") ?? {});
            mockPetEnabled = Boolean(next.pet_enabled);
            mockPetDirectory = String(next.pet_directory ?? "");
            mockLocale = String(next.locale ?? mockLocale);
            (window as any).__lastSetSettings = next;
            return null;
          }
          case "check_for_updates":
            if (mockUpdateCheckPending) {
              await new Promise<void>((resolve) => {
                resolveMockUpdateCheck = resolve;
              });
              mockUpdateCheckPending = false;
            }
            if (mockUpdateCheckError) {
              const error = mockUpdateCheckError;
              mockUpdateCheckError = null;
              throw error;
            }
            return mockUpdateCheck;
          case "download_update": {
            const onEvent = arg("onEvent") as Channel | undefined;
            onEvent?.onmessage?.({
              event: "started",
              data: { content_length: 100 },
            });
            onEvent?.onmessage?.({
              event: "progress",
              data: { chunk_length: 25 },
            });
            if (mockUpdateDownloadPending) {
              (window as any).__mockUpdateProgress = (chunkLength: number) => onEvent?.onmessage?.({
                event: "progress",
                data: { chunk_length: chunkLength },
              });
              await new Promise<void>((resolve) => {
                resolveMockUpdateDownload = resolve;
              });
              delete (window as any).__mockUpdateProgress;
              mockUpdateDownloadPending = false;
            }
            if (mockUpdateDownloadError) {
              const error = mockUpdateDownloadError;
              mockUpdateDownloadError = null;
              throw error;
            }
            onEvent?.onmessage?.({
              event: "progress",
              data: { chunk_length: 75 },
            });
            onEvent?.onmessage?.({ event: "verified" });
            return null;
          }
          case "install_update":
            if (mockInstallUpdateError) {
              const error = mockInstallUpdateError;
              mockInstallUpdateError = null;
              throw error;
            }
            (window as any).__mockUpdateInstalled = true;
            return null;
          case "validate_settings": {
            const validationSettings = plain(arg("settings") ?? {});
            const validatedModel = String(validationSettings.model ?? "");
            if (validatedModel === "gpt-image-2") {
              return "Validated openai_responses with gpt-image-2";
            }
            if (validatedModel === "grok-imagine-image-2.0") {
              return "Validated openai with grok-imagine-image-2.0";
            }
            if (validatedModel.startsWith("grok-imagine-video")) {
              return `Validated openai with ${validatedModel}`;
            }
            return "Validated openai with deepseek-v4-pro";
          }
          case "get_memory_view":
            return memoryViewFor(resolveMemoryProjectId(args, arg));
          case "set_memory_enabled":
            memoryEnabled = !!(arg("enabled") ?? args?.enabled);
            return memoryViewFor(resolveMemoryProjectId(args, arg));
          case "get_auto_failure_analysis_settings":
            return { ...autoFailureAnalysis };
          case "set_auto_failure_analysis_settings":
            autoFailureAnalysis = {
              ...autoFailureAnalysis,
              ...plain(arg("settings") ?? {}),
            };
            return { ...autoFailureAnalysis };
          case "propose_turn_memory": {
            const sessionId = String(arg("sessionId") ?? "");
            const automatic = Boolean(arg("automatic"));
            const latest = lastMessageBySession[sessionId] ?? "";
            if (automatic && !autoFailureAnalysis.enabled && !/记住|REMEMBER/i.test(latest)) {
              return null;
            }
            if (automatic && !/记住|REMEMBER|TOOLFAILMEMORY/i.test(latest)) {
              return null;
            }
            const failure = /TOOLFAILMEMORY/i.test(latest);
            return {
              session_id: sessionId,
              turn_index: Number(arg("turnIndex") ?? 0),
              scope: /记住|REMEMBER/i.test(latest) ? "global" : "project",
              content: failure
                ? "Two shell calls failed because the input path was invalid; validate the path before retrying."
                : "Prefer reproducible local workflows for this project.",
              trigger: failure ? "tool_failures" : (automatic ? "explicit" : "manual"),
              tool_calls: failure ? 3 : 1,
              failed_tool_calls: failure ? 2 : 0,
              failure_rate: failure ? 66.7 : 0,
              global_memories: globalMemories,
            };
          }
          case "confirm_turn_memory": {
            if (String(arg("scope")) === "global") {
              const replaceId = String(arg("replaceId") ?? "");
              const existing = globalMemories.find((memory) => memory.id === replaceId);
              if (existing) {
                existing.content = String(arg("content") ?? "");
              } else {
                globalMemories.push({
                  id: `global-memory-${globalMemories.length + 1}`,
                  content: String(arg("content") ?? ""),
                });
              }
            }
            return {
              id: String(arg("scope")) === "global"
                ? String(arg("replaceId") ?? "global-memory-1")
                : null,
              scope: String(arg("scope") ?? "project"),
            };
          }
          case "create_global_memory": {
            const memory = {
              id: `global-memory-${globalMemories.length + 1}`,
              content: String(arg("content") ?? ""),
            };
            globalMemories.unshift(memory);
            return memory;
          }
          case "update_global_memory": {
            const existing = globalMemories.find(
              (memory) => memory.id === String(arg("id") ?? ""),
            );
            if (existing) existing.content = String(arg("content") ?? "");
            return null;
          }
          case "delete_global_memory":
            globalMemories = globalMemories.filter((memory) => memory.id !== String(arg("id") ?? ""));
            return null;
          case "get_auto_review_enabled":
            return autoReviewEnabled;
          case "set_auto_review_enabled":
            autoReviewEnabled = !!args?.enabled;
            return autoReviewEnabled;
          case "get_session_delegation_enabled":
            return sessionDelegationEnabled[String(arg("sessionId") ?? "")] ?? false;
          case "set_session_delegation_enabled": {
            const sessionId = String(arg("sessionId") ?? "");
            lastDelegationSessionId = sessionId;
            sessionDelegationEnabled[sessionId] = Boolean(arg("enabled"));
            for (const snapshot of mockAgentWorkflows) {
              if (snapshot.workflow.frame_id === sessionId) {
                snapshot.delegation_enabled = sessionDelegationEnabled[sessionId];
              }
            }
            return sessionDelegationEnabled[sessionId];
          }
          case "get_session_plan_mode": {
            const sessionId = String(arg("sessionId") ?? "");
            return acpBindings[sessionId] ? null : sessionPlanMode[sessionId] ?? false;
          }
          case "set_session_plan_mode": {
            const sessionId = String(arg("sessionId") ?? "");
            if (acpBindings[sessionId]) {
              throw new Error("This conversation is bound to an ACP agent; use its own plan mode.");
            }
            sessionPlanMode[sessionId] = Boolean(arg("enabled"));
            return sessionPlanMode[sessionId];
          }
          case "get_session_full_permission": {
            const sessionId = String(arg("sessionId") ?? "");
            return sessionFullPermission[sessionId] ?? false;
          }
          case "set_session_full_permission": {
            const sessionId = String(arg("sessionId") ?? "");
            sessionFullPermission[sessionId] = Boolean(arg("enabled"));
            return sessionFullPermission[sessionId];
          }
          case "get_session_agent_completion": {
            const sessionId = String(arg("sessionId") ?? "");
            return sessionAgentCompletion[sessionId] ?? { policy: "inline", auto_resume: false };
          }
          case "set_session_agent_completion": {
            const sessionId = String(arg("sessionId") ?? "");
            const policy = arg("policy") === "background" ? "background" : "inline";
            const value = {
              policy,
              auto_resume: policy === "background" && Boolean(arg("autoResume")),
            } as const;
            sessionAgentCompletion[sessionId] = value;
            return value;
          }
          case "write_memory_file": {
            const name = String(arg("name") ?? "");
            const content = String(arg("content") ?? "");
            const files = memoryFilesFor(resolveMemoryProjectId(args, arg));
            const existing = files.find((file) => file.name === name);
            if (existing) {
              existing.preview = content.slice(0, 240);
              existing.bytes = content.length;
            } else if (name) {
              files.push({ name, preview: content.slice(0, 240), bytes: content.length });
            }
            return files;
          }
          case "delete_memory_file": {
            const projectId = resolveMemoryProjectId(args, arg);
            memoryByProject[projectId] = memoryFilesFor(projectId).filter(
              (file) => file.name !== arg("name"),
            );
            return memoryFilesFor(projectId);
          }
          case "clear_memory": {
            const projectId = resolveMemoryProjectId(args, arg);
            memoryByProject[projectId] = [];
            return memoryFilesFor(projectId);
          }
          case "read_memory_file":
            return memoryFilesFor(resolveMemoryProjectId(args, arg))
              .find((file) => file.name === arg("name"))?.preview ?? "";
          case "new_session": {
            if (failNextNewSession) {
              const message = failNextNewSession;
              failNextNewSession = null;
              throw new Error(message);
            }
            const id = `s-${Math.random().toString(36).slice(2)}`;
            sessionModels[id] = activeHttpModelId();
            return id;
          }
          case "start_scratch_chat": {
            scratchOpen = true;
            scratchSessionId = `scratch-${Math.random().toString(36).slice(2)}`;
            sessionModels[scratchSessionId] = activeHttpModelId();
            ((window as any).__scratchOpenEvents ??= []).push(true);
            return { sessionId: scratchSessionId, projectId: "scratch:mock" };
          }
          case "close_scratch_chat": {
            scratchOpen = false;
            if (scratchSessionId) {
              delete sessionModels[scratchSessionId];
            }
            scratchSessionId = null;
            ((window as any).__scratchOpenEvents ??= []).push(false);
            return null;
          }
          case "branch_session": {
            const source = String(arg("sessionId") ?? "");
            const sourceSession = mockSessions.find((session) => session.id === source);
            if (sourceSession?.branched_from || sourceSession?.branch_state) {
              throw new Error("Conversation branches cannot be branched again.");
            }
            const id = `branch-${Math.random().toString(36).slice(2)}`;
            sessionModels[id] = sessionModels[source] ?? activeHttpModelId();
            if (Object.prototype.hasOwnProperty.call(sessionReasoningEfforts, source)) {
              sessionReasoningEfforts[id] = sessionReasoningEfforts[source];
            }
            if (Object.prototype.hasOwnProperty.call(sessionServiceTiers, source)) {
              sessionServiceTiers[id] = sessionServiceTiers[source];
            }
            return id;
          }
          case "preview_session_branch_merge": {
            const branch = mockSessions.find((session) => session.id === arg("id"));
            if (!branch?.branched_from) throw new Error("Conversation is not a mergeable branch");
            if (branch.branch_state === "merged") throw new Error("Conversation branch has already been merged");
            return {
              main_session_id: branch.branched_from,
              branch_session_id: branch.id,
              branch_title: String(branch.title).replace(/^Branch: /, ""),
              checkpoint_user_index: 0,
              checkpoint_kind: "after_response",
              guard_hash: "mock-branch-merge-guard",
              new_message_count: 2,
              messages: [
                { seq: 3, role: "user", text: `${branch.title} task` },
                { seq: 4, role: "assistant", text: `${branch.title} result` },
              ],
            };
          }
          case "summarize_session_branch_merge":
            return arg("userGuidance")
              ? `Guided version: ${arg("userGuidance")}`
              : "The branch completed its focused analysis and produced a reusable result.";
          case "merge_session_branch_summary": {
            const branch = mockSessions.find((session) => session.id === arg("id"));
            if (!branch?.branched_from) throw new Error("Conversation is not a mergeable branch");
            if (branch.branch_state === "merged") throw new Error("Conversation branch has already been merged");
            mockBranchMergedSummary = String(arg("summary") ?? "");
            branch.branch_state = "merged";
            return {
              main_session_id: branch.branched_from,
              branch_session_id: branch.id,
              summary_message_seq: 5,
            };
          }
          case "preview_turn_undo":
          case "undo_turn":
            return {
              restoreFiles: ["notes.md"],
              removeFiles: ["summary.md"],
              removeArtifacts: ["summary.md"],
              unsupportedFiles: ["paper.docx"],
              conflicts: [],
            };
          case "list_artifacts":
            return mockPublication ? [artifacts[1]] : [];
          case "download_artifact":
          case "download_artifact_version":
            return null;
          case "side_chat": {
            const question = String(arg("question") ?? "");
            if (question === "SIDESCROLLTEST") {
              return {
                answer: Array.from(
                  { length: 40 },
                  (_, index) => `Side answer line ${index + 1}`,
                ).join("\n\n"),
                sessionId: String(arg("sessionId") ?? ""),
                snapshotVersion: 12,
                evidence: [],
                noEvidence: false,
              };
            }
            if (question === "NO_EVIDENCE_TEST") {
              return {
                answer: "",
                sessionId: String(arg("sessionId") ?? ""),
                snapshotVersion: 12,
                evidence: [],
                noEvidence: true,
              };
            }
            return {
              answer: `Side answer: ${question} [S1]`,
              sessionId: String(arg("sessionId") ?? ""),
              snapshotVersion: 12,
              evidence: [{
                sourceId: "event-7",
                eventSeq: 7,
                messageSeq: null,
                turn: 2,
                role: "assistant",
                excerpt: "The main thread recorded this evidence.",
                relevance: "Matched the question",
              }],
              noEvidence: false,
            };
          }
          case "confirm_response": {
            const frameId = String(arg("sessionId") ?? "");
            const resolve = nativeConfirmResolvers[frameId];
            if (resolve) {
              delete nativeConfirmResolvers[frameId];
              (window as any).__nativeConfirmPending[frameId] = false;
              emit("agent", {
                kind: "ToolResult",
                frame_id: frameId,
                name: "shell",
                ok: Boolean(arg("approved")),
                content: arg("approved") ? "approved" : "denied",
              });
              emit("agent", { kind: "Done", frame_id: frameId, stop_reason: "end_turn" });
              resolve(frameId);
            }
            return null;
          }
          case "dismiss_onboarding":
            return null;
          case "stop_agent":
            if ((window as any).__failStopAgent) {
              throw new Error("stop command unavailable");
            }
            if ((window as any).__holdStopAgent) {
              return null;
            }
            setTimeout(() => {
              const frameId = String(arg("id") ?? arg("sessionId") ?? "");
              emit("agent", { kind: "Done", frame_id: frameId, stop_reason: "cancelled" });
              acpLongResolvers[frameId]?.(frameId);
              delete acpLongResolvers[frameId];
            }, 0);
            return null;
          case "send_message": {
            const fid = String(arg("sessionId") ?? arg("session_id") ?? "") || "t1";
            const msg = String(arg("message") ?? "");
            lastMessageBySession[fid] = msg;
            sessionModels[fid] ??= activeHttpModelId();
            const acpAgentId = arg("acpAgentId") ?? acpBindings[fid];
            if (acpAgentId && String(msg).includes("ACPTHINK")) {
              // Codex-style ordering: visible commentary streams first, then
              // reasoning, then tool activity. The UI must preserve those as
              // separate transcript layers.
              acpBindings[fid] = acpAgentId;
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "Let me search the literature first." });
                emit("agent", { kind: "Reasoning", frame_id: fid, delta: "Planning which databases to query." });
                emit("acp-session-update", { frameId: fid, kind: "ToolCall", payload: { toolCallId: "s1", title: "web_search", kind: "search", status: "in_progress" } });
                emit("acp-session-update", { frameId: fid, kind: "ToolCallUpdate", payload: { toolCallId: "s1", status: "completed", content: [{ type: "content", content: { type: "text", text: "hit" } }] } });
                emit("agent", { kind: "Done", frame_id: fid, stop_reason: "end_turn" });
              }, 30);
              return fid;
            }
            if (acpAgentId) {
              acpBindings[fid] = acpAgentId;
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("acp-session-state", {
                  frameId: fid,
                  modes: { currentModeId: "agent", availableModes: [{ id: "read-only", name: "Read Only" }, { id: "agent", name: "Agent" }, { id: "full-access", name: "Full Access" }] },
                  configOptions: [{ id: "model", name: "Model", type: "select", currentValue: "fast", options: [{ value: "fast", name: "Fast" }, { value: "smart", name: "Smart" }] }],
                });
                emit("acp-session-update", { frameId: fid, kind: "ToolCall", payload: { toolCallId: "tool-a", title: "Read files", kind: "read", status: "in_progress" } });
                emit("acp-session-update", { frameId: fid, kind: "ToolCall", payload: { toolCallId: "tool-b", title: "Run checks", kind: "execute", status: "in_progress" } });
                emit("acp-session-update", { frameId: fid, kind: "ToolCallUpdate", payload: { toolCallId: "tool-a", status: "completed", content: [{ type: "content", content: { type: "text", text: "read complete" } }] } });
                emit("acp-session-update", { frameId: fid, kind: "Plan", payload: { entries: [{ content: "Inspect", priority: "high", status: "completed" }, { content: "Implement", priority: "medium", status: "in_progress" }] } });
                emit("acp-session-update", { frameId: fid, kind: "ConfigOptions", payload: { configOptions: [
                  { id: "model", name: "Model", type: "select", currentValue: "smart", options: [{ value: "fast", name: "Fast" }, { value: "smart", name: "Smart" }] },
                  { id: "fast_mode", name: "Fast Mode", type: "boolean", currentValue: false },
                ] } });
                emit("acp-session-update", { frameId: fid, kind: "Usage", payload: { used: 1200, size: 8000 } });
                if (String(msg).includes("PERMISSION")) {
                  acpPermissionFrames["permission-1"] = fid;
                  emit("permission-request", { requestId: "permission-1", frameId: fid, toolCall: { toolCallId: "tool-b", title: "Run checks" }, options: [{ id: "allow", name: "Allow once", kind: "allowonce" }, { id: "reject", name: "Reject", kind: "rejectonce" }] });
                }
                if (String(msg).includes("ASKUSER")) {
                  askUserFrames["ask-1"] = fid;
                  emit("ask-user-request", { requestId: "ask-1", frameId: fid, payload: {
                    v: 1, source: "acp", question: "Which aligner should the pipeline use?", allow_freeform: true,
                    options: [{ label: "STAR", description: "splice-aware, needs more RAM" }, { label: "HISAT2", description: "lighter" }],
                  } });
                }
                emit("agent", { kind: "Text", frame_id: fid, delta: "Hello from ACP." });
                if (!String(msg).includes("LONG") && !String(msg).includes("PERMISSION")) emit("agent", { kind: "Done", frame_id: fid, stop_reason: "end_turn" });
              }, 30);
              if (String(msg).includes("LONG")) return await new Promise<string>((resolve) => { acpLongResolvers[fid] = resolve; });
              return fid;
            }
            if (String(msg).includes("PRESTARTFAIL")) {
              throw new Error("No model profile is available");
            }
            if (String(msg).includes("TOOLFAILMEMORY")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "ToolCall", frame_id: fid, name: "shell", preview: "first" });
                emit("agent", { kind: "ToolResult", frame_id: fid, name: "shell", ok: false, content: "path not found" });
                emit("agent", { kind: "ToolCall", frame_id: fid, name: "shell", preview: "second" });
                emit("agent", { kind: "ToolResult", frame_id: fid, name: "shell", ok: false, content: "invalid path" });
                emit("agent", { kind: "ToolCall", frame_id: fid, name: "shell", preview: "third" });
                emit("agent", { kind: "ToolResult", frame_id: fid, name: "shell", ok: true, content: "ok" });
                emit("agent", { kind: "Text", frame_id: fid, delta: "Recovered after validating the path." });
                emit("agent", { kind: "Done", frame_id: fid, stop_reason: "end_turn" });
              }, 30);
              return fid;
            }
            if (String(msg).includes("POSTSTARTFAIL_EVENT")) {
              // Mirrors the real backend: live Error event + turn-started
              // rejection (plain message only on the event card).
              return await new Promise<string>((_resolve, reject) => {
                setTimeout(() => {
                  emit("agent", { kind: "User", frame_id: fid, text: msg });
                  emit("agent", {
                    kind: "Error",
                    frame_id: fid,
                    message: 'api: 400 {"error":{"message":"max_tokens too high"}}',
                  });
                  reject(
                    new Error(
                      '[turn-started] api: 400 {"error":{"message":"max_tokens too high"}}',
                    ),
                  );
                }, 30);
              });
            }
            if (String(msg).includes("POSTSTARTFAIL")) {
              throw new Error("[turn-started] execution failed after turn/start");
            }
            if (String(msg).includes("AUTOCONTINUE")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "First segment. " });
                emit("agent", { kind: "Compaction", frame_id: fid, before: 1, after: 10, strategy: "auto_continue" });
                emit("agent", { kind: "Text", frame_id: fid, delta: "Final segment." });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(msg).includes("SHARETABLEMATH")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", {
                  kind: "Text",
                  frame_id: fid,
                  delta: "| 项目 | 数值 |\n| --- | --- |\n| A | 1 |\n| B | 2 |\n\n质能方程 $E = mc^2$\n\n$$\\int_0^1 x^2 dx$$",
                });
                emit("agent", { kind: "Done", frame_id: fid, stop_reason: "end_turn" });
              }, 30);
              return fid;
            }
            if (String(msg).includes("SHARETHINK")) {
              // Fixture for /share: a turn with a visible thinking block, so
              // the share dialog lists it (deselected by default). The reply
              // carries Markdown (heading, list, code fence) so the exported
              // PNG exercises the rendered-Markdown path.
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Reasoning", frame_id: fid, delta: "Secret plan: verify with Alice first." });
                emit("agent", { kind: "Text", frame_id: fid, delta: "Alice confirmed the **spectrum** is clean.\n\n## Fit summary\n- peak A at 530 nm\n- peak B at 612 nm\n\n```python\nfit(spectrum)\n```" });
                emit("agent", { kind: "Done", frame_id: fid, stop_reason: "end_turn" });
              }, 30);
              return fid;
            }
            if (String(msg).includes("TOOLONLYDONE")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "I will record the decision first." });
                emit("agent", { kind: "ToolCall", frame_id: fid, name: "research_graph", preview: "record_decision" });
                emit("agent", { kind: "ToolResult", frame_id: fid, name: "research_graph", ok: true, content: '{"node_id":"decision-1"}' });
                // Regression fixture: an old/buggy backend mislabeled the
                // provider cut after this tool as a successful turn.
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(msg).includes("MONITORRUN")) {
              return await new Promise<string>((resolve) => {
                monitorRunFrameId = fid;
                resolveMonitorRun = resolve;
                // The monitored run belongs to the session that submitted it,
                // like the real backend records frame_id at submission.
                const monitored = runs.find((item) => item.id === "run-local-002");
                if (monitored) monitored.frame_id = fid;
                setTimeout(() => {
                  emit("agent", { kind: "User", frame_id: fid, text: msg });
                  emit("agent", { kind: "Reasoning", frame_id: fid, delta: "Attach the existing Run monitor." });
                  emit("agent", { kind: "ToolCall", frame_id: fid, name: "monitor_run", preview: "run-local-002" });
                }, 30);
              });
            }
            if (String(msg).includes("IMAGEGENPLACEHOLDER")) {
              return await new Promise<string>((resolve) => {
                setTimeout(() => {
                  emit("agent", { kind: "User", frame_id: fid, text: msg });
                  emit("agent", {
                    kind: "Text",
                    frame_id: fid,
                    delta: "I’ll generate the scientific figure now.",
                  });
                  emit("agent", {
                    kind: "ToolCall",
                    frame_id: fid,
                    name: "generate_image",
                    preview: "figures/pathway.png",
                  });
                }, 30);
                setTimeout(() => {
                  emit("agent", {
                    kind: "ToolResult",
                    frame_id: fid,
                    name: "generate_image",
                    ok: true,
                    content: "Generated PNG at figures/pathway.png.",
                  });
                }, 1_200);
                setTimeout(() => {
                  emit("agent", {
                    kind: "Text",
                    frame_id: fid,
                    delta: "The scientific figure is ready.",
                  });
                  emit("agent", { kind: "Done", frame_id: fid, stop_reason: "end_turn" });
                  resolve(fid);
                }, 1_350);
              });
            }
            if (String(msg).includes("VIDEOGENPLACEHOLDER")) {
              return await new Promise<string>((resolve) => {
                setTimeout(() => {
                  emit("agent", { kind: "User", frame_id: fid, text: msg });
                  emit("agent", {
                    kind: "Text",
                    frame_id: fid,
                    delta: "I’ll generate the demo clip now.",
                  });
                  emit("agent", {
                    kind: "ToolCall",
                    frame_id: fid,
                    name: "generate_video",
                    preview: "media/demo.mp4",
                  });
                }, 30);
                setTimeout(() => {
                  emit("agent", {
                    kind: "ToolResult",
                    frame_id: fid,
                    name: "generate_video",
                    ok: true,
                    content: "Generated MP4 at media/demo.mp4.",
                  });
                }, 1_200);
                setTimeout(() => {
                  emit("agent", {
                    kind: "Text",
                    frame_id: fid,
                    delta: "The demo clip is ready.",
                  });
                  emit("agent", { kind: "Done", frame_id: fid, stop_reason: "end_turn" });
                  resolve(fid);
                }, 1_350);
              });
            }
            // Long-approval path (#63 regression test): emit a confirm-request
            // whose body is far taller than the viewport.
            if (String(arg("message") ?? "").includes("NEEDPLAN")) {
              setTimeout(
                () =>
                  emit("confirm-request", {
                    frame_id: fid,
                    message: "Review the proposed plan",
                    tool: "update_plan",
                    preview: "[x] Inspect the evidence\n[~] Implement the change\n[ ] Run verification",
                  }),
                50,
              );
              return fid;
            }
            if (String(arg("message") ?? "").includes("BLOCKINGCONFIRM")) {
              setTimeout(
                () =>
                  emit("confirm-request", {
                    frame_id: fid,
                    message: "Dangerous command detected:\nRemove generated files?",
                    tool: "shell",
                    preview: "Remove-Item generated.tmp",
                  }),
                50,
              );
              (window as any).__nativeConfirmPending[fid] = true;
              return await new Promise<string>((resolve) => {
                nativeConfirmResolvers[fid] = resolve;
              });
            }
            if (String(arg("message") ?? "").includes("NEEDCONFIRM")) {
              const longBody = Array.from({ length: 120 }, (_, i) => `rm -rf /mock/path/line-${i}`).join("\n");
              setTimeout(
                () =>
                  emit("confirm-request", {
                    frame_id: fid,
                    message: `Dangerous command detected:\n${longBody}`,
                    tool: "shell",
                    preview: longBody,
                  }),
                50,
              );
              return fid;
            }
            if (String(arg("message") ?? "").includes("NEEDRCONFIRM")) {
              setTimeout(
                () =>
                  emit("confirm-request", {
                    frame_id: fid,
                    message: "R execution requires approval",
                    tool: "r",
                    preview: "[r @ local] summary(dataset)",
                  }),
                50,
              );
              return fid;
            }
            // Slow stream keeps send_message pending until Done. This mirrors the
            // native command lifecycle and leaves enough live time to assert that
            // Markdown/projection work is deferred between token batches.
            if (String(arg("message") ?? "").includes("MARKDOWNSTREAM")) {
              return await new Promise<string>((resolve) => {
                let n = 0;
                const tick = () => {
                  if (n < 24) {
                    // Cross both adaptive Markdown thresholds so the browser
                    // test observes a formatted prefix and a cheap live tail.
                    emit("agent", {
                      kind: "Text",
                      frame_id: fid,
                      delta: `**stream line ${n}** ${"x".repeat(1_500)}\n`,
                    });
                    n++;
                    setTimeout(tick, 50);
                  } else {
                    emit("agent", { kind: "Done", frame_id: fid });
                    resolve(fid);
                  }
                };
                setTimeout(() => {
                  emit("agent", { kind: "User", frame_id: fid, text: msg });
                  tick();
                }, 30);
              });
            }
            // Long-conversation tool returns (#927): large tool bodies remount
            // the live step row and briefly collapse the thread. Follow-bottom
            // must survive those rebuilds. Checked before SCROLLTEST because
            // that name is a substring of this one.
            if (String(arg("message") ?? "").includes("TOOLSCROLLTEST")) {
              const bulky = (n: number) => Array.from(
                { length: 40 },
                (_, i) => `tool result block ${n} line ${i} ${"x".repeat(80)}`,
              ).join("\n");
              return new Promise<string>((resolve) => {
                setTimeout(() => {
                  emit("agent", { kind: "Text", frame_id: fid, delta: "I’ll inspect the files next.\n" });
                  emit("agent", { kind: "ToolCall", frame_id: fid, name: "read", preview: "matrix.tsv" });
                }, 20);
                setTimeout(() => {
                  emit("agent", { kind: "ToolResult", frame_id: fid, name: "read", ok: true, content: bulky(1) });
                  emit("agent", { kind: "ToolCall", frame_id: fid, name: "python", preview: "df.describe()" });
                }, 80);
                setTimeout(() => {
                  emit("agent", { kind: "ToolResult", frame_id: fid, name: "python", ok: true, content: bulky(2) });
                  emit("agent", { kind: "Text", frame_id: fid, delta: "Tools finished at the tail." });
                  emit("agent", { kind: "Done", frame_id: fid });
                  resolve(fid);
                }, 200);
              });
            }
            // Long-stream path (#61 regression test): drip many text deltas so the
            // thread re-renders repeatedly and grows well past the viewport.
            if (String(arg("message") ?? "").includes("SCROLLTEST")) {
              let n = 0;
              const tick = () => {
                if (n < 80) {
                  emit("agent", { kind: "Text", frame_id: fid, delta: `line ${n}\n` });
                  n++;
                  setTimeout(tick, 6);
                } else {
                  emit("agent", { kind: "Done", frame_id: fid });
                }
              };
              setTimeout(tick, 20);
              return fid;
            }
            if (String(arg("message") ?? "").includes("DELAYUSER")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "delayed reply" });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 1200);
              return fid;
            }
            if (String(arg("message") ?? "").includes("REVIEWBASE")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "Earlier answer." });
                emit("agent", {
                  kind: "Usage",
                  frame_id: fid,
                  round: 1,
                  input: 100,
                  output: 10,
                  reasoning: 0,
                  cached: 0,
                  ctx_tokens: 110,
                  max_context: 8_000,
                });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("CONTEXTUSAGELEGACY")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "Legacy usage totals only." });
                emit("agent", {
                  kind: "Usage",
                  frame_id: fid,
                  round: 1,
                  input: 25_000,
                  output: 400,
                  reasoning: 0,
                  cached: 0,
                  ctx_tokens: 25_400,
                  max_context: 1_000_000,
                });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("CONTEXTUSAGERUNNING")) {
              return await new Promise<string>((resolve) => {
                setTimeout(() => {
                  emit("agent", { kind: "User", frame_id: fid, text: msg });
                  emit("agent", { kind: "Text", frame_id: fid, delta: "Still running on the original model." });
                  emit("agent", {
                    kind: "Usage",
                    frame_id: fid,
                    round: 1,
                    input: 79_200,
                    output: 700,
                    reasoning: 0,
                    cached: 50_000,
                    ctx_tokens: 79_900,
                    max_context: 128_000,
                  });
                }, 30);
                setTimeout(() => {
                  emit("agent", { kind: "Done", frame_id: fid });
                  resolve(fid);
                }, 1_200);
              });
            }
            if (String(arg("message") ?? "").includes("CONTEXTUSAGEWARN")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "Context is getting full." });
                emit("agent", {
                  kind: "Usage",
                  frame_id: fid,
                  round: 1,
                  input: 90_000,
                  output: 1_520,
                  reasoning: 0,
                  cached: 0,
                  ctx_tokens: 91_520,
                  max_context: 128_000,
                });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("CONTEXTUSAGEDANGER")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "Context is almost full." });
                emit("agent", {
                  kind: "Usage",
                  frame_id: fid,
                  round: 1,
                  input: 114_000,
                  output: 1_840,
                  reasoning: 0,
                  cached: 0,
                  ctx_tokens: 115_840,
                  max_context: 128_000,
                });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("CONTEXTUSAGE")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "Context usage is ready." });
                emit("agent", {
                  kind: "Usage",
                  frame_id: fid,
                  round: 1,
                  input: 79_200,
                  output: 700,
                  reasoning: 0,
                  cached: 50_000,
                  ctx_tokens: 79_900,
                  max_context: 300_000,
                  context_usage: {
                    system_prompt: 6_000,
                    tool_definitions: 22_700,
                    rules: 2_200,
                    skills: 6_100,
                    mcp_dynamic_tools: 4_200,
                    subagent_definitions: 2_400,
                    conversation: 36_300,
                  },
                });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("AUTOREVIEWUNREVIEWABLE")) {
              const incompleteReport = {
                id: "review-auto-unreviewable",
                summary: "Review could not establish full traceability because tool output evidence was incomplete.",
                reviewer_model: "Test ACP Agent",
                reviewer_effort: "",
                reviewer_backend: "acp_agent",
                review_status: "unreviewable",
                evidence_coverage: 0,
                coverage_gaps: ["python analysis.py did not persist inspectable output (only status, location, or terminal handle)."],
                findings: [],
              };
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "The ACP analysis completed." });
                emit("agent", { kind: "ReviewStarted", frame_id: fid });
                emit("agent", { kind: "Review", frame_id: fid, report: incompleteReport });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("AUTOREVIEWFAIL")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "The primary answer still completed." });
                emit("agent", { kind: "ReviewStarted", frame_id: fid });
                emit("agent", { kind: "ReviewFailed", frame_id: fid, message: "ACP reviewer returned invalid JSON" });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("AUTOREVIEWCLEAN")) {
              const cleanReport = {
                id: "review-auto-clean",
                summary: "No issues found in the response.",
                reviewer_model: "claude-sonnet-5",
                reviewer_effort: "high",
                findings: [],
              };
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "The analysis is consistent with the tool result." });
                emit("agent", { kind: "ReviewStarted", frame_id: fid });
                emit("agent", { kind: "Review", frame_id: fid, report: cleanReport });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("AUTOREVIEW")) {
              const openReport = {
                id: "review-auto-1",
                summary: "Checked the reported value against the tool result.",
                reviewer_model: "claude-sonnet-5",
                reviewer_effort: "high",
                findings: [
                  {
                    message_index: 3,
                    claim: "The analysis reports 5 significant genes.",
                    evidence: "The tool result reports 3 significant genes.",
                    fix: "Change the count from 5 to 3.",
                    verdict: "warn",
                    severity: "low",
                    status: "open",
                  },
                ],
              };
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: "The analysis found 5 significant genes." });
                emit("agent", { kind: "ReviewStarted", frame_id: fid });
                emit("agent", { kind: "Review", frame_id: fid, report: openReport });
                emit("agent", { kind: "CorrectionStarted", frame_id: fid, model: "deepseek-v4-pro" });
                emit("agent", { kind: "Text", frame_id: fid, delta: "Correction: the analysis found 3 significant genes." });
                emit("agent", {
                  kind: "Review",
                  frame_id: fid,
                  report: {
                    ...openReport,
                    summary: "The corrected value matches the tool result.",
                    findings: openReport.findings.map((finding) => ({ ...finding, status: "resolved" })),
                  },
                });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("STEPSLIVE")) {
              return await new Promise<string>((resolve) => {
                setTimeout(() => {
                  emit("agent", { kind: "User", frame_id: fid, text: msg });
                  emit("agent", { kind: "Reasoning", frame_id: fid, delta: "Inspect the live output." });
                  emit("agent", { kind: "ToolCall", frame_id: fid, name: "shell", preview: "long-running-command" });
                }, 30);
                setTimeout(() => {
                  emit("agent", { kind: "ToolResult", frame_id: fid, name: "shell", ok: true, content: "shell output line" });
                }, 2_500);
                setTimeout(() => {
                  emit("agent", { kind: "ToolCall", frame_id: fid, name: "python", preview: "print('next')" });
                  emit("agent", { kind: "ToolResult", frame_id: fid, name: "python", ok: true, content: "next output" });
                }, 2_800);
                setTimeout(() => {
                  emit("agent", { kind: "Text", frame_id: fid, delta: "Live steps finished." });
                  emit("agent", { kind: "Done", frame_id: fid });
                  resolve(fid);
                }, 3_100);
              });
            }
            if (String(arg("message") ?? "").includes("RZSTREAM")) {
              // Staggered reasoning deltas keep rebuilding the fingerprint-keyed
              // chat row; the expanded thinking block must not snap shut.
              // `Done` settles the turn and moves the reasoning into the steps
              // group (details.rz disappears), so it stays far out: on slow CI
              // runners the click+assert chain itself can take several seconds,
              // and the test must finish it inside the live window.
              return await new Promise<string>((resolve) => {
                setTimeout(() => {
                  emit("agent", { kind: "User", frame_id: fid, text: msg });
                  emit("agent", { kind: "Reasoning", frame_id: fid, delta: "First thought." });
                }, 30);
                setTimeout(() => {
                  emit("agent", { kind: "Reasoning", frame_id: fid, delta: " More reasoning arrives." });
                }, 3_000);
                setTimeout(() => {
                  emit("agent", { kind: "Text", frame_id: fid, delta: "Stream done." });
                  emit("agent", { kind: "Done", frame_id: fid });
                  resolve(fid);
                }, 12_000);
              });
            }
            if (String(arg("message") ?? "").includes("RAUTOREFRESH")) {
              // An agent `r` cell lazily revives the dead local R runtime and
              // finishes; the open memory environment must follow without a
              // manual sync click.
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "ToolCall", frame_id: fid, name: "r", preview: "[r @ local] x <- rnorm(10)" });
                const local = runtimeInfos.find((item) =>
                  item.key.projectId === "default"
                  && item.key.contextId === "local"
                  && item.key.language === "r"
                );
                if (local) {
                  local.runtimeId = `runtime-r-local-${Date.now()}`;
                  local.generation += 1;
                  local.status = "ready";
                  local.processId = 5501;
                  local.lastActivityAtMs = Date.now();
                  local.lastError = null;
                }
                emit("agent", { kind: "ToolResult", frame_id: fid, name: "r", ok: true, content: "(no output)" });
                emit("agent", { kind: "Text", frame_id: fid, delta: "Created x in the R session." });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("RNOTEBOOK")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "ToolCall", frame_id: fid, name: "r", preview: "[r @ ssh:gpu-server] summary(dataset)" });
                emit("agent", { kind: "ToolResult", frame_id: fid, name: "r", ok: true, content: "Length Class Mode" });
                emit("agent", { kind: "Text", frame_id: fid, delta: "R summary complete." });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("CLIPBOARDIMAGE")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", {
                  kind: "Text",
                  frame_id: fid,
                  delta: [
                    "Saved the screenshots as local files for this turn.",
                    "",
                    '<image name="clipboard-preview.png" path="C:\\Users\\Alice\\AppData\\Local\\Temp\\clipboard-preview.png"></image>',
                    "",
                    '<image name="clipboard-preview-2.png" path="C:\\Users\\Alice\\AppData\\Local\\Temp\\clipboard-preview-2.png"></image>',
                  ].join("\n"),
                });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("ARTIFACTATTRIBUTION")) {
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "ToolCall", frame_id: fid, name: "shell", preview: "Get-ChildItem" });
                emit("agent", {
                  kind: "ToolResult",
                  frame_id: fid,
                  name: "shell",
                  ok: true,
                  content: "old.csv\nplots/old.png\nnotes/old-report.md",
                });
                emit("agent", { kind: "ToolCall", frame_id: fid, name: "write", preview: "results/new.png" });
                emit("agent", {
                  kind: "FileChanged",
                  frame_id: fid,
                  path: "/mock/root/results/new.png",
                });
                emit("agent", {
                  kind: "ToolResult",
                  frame_id: fid,
                  name: "write",
                  ok: true,
                  content: "write completed",
                });
                emit("agent", {
                  kind: "Text",
                  frame_id: fid,
                  delta: "I will verify the generated output first.",
                });
                emit("agent", {
                  kind: "ToolCall",
                  frame_id: fid,
                  name: "view_image",
                  preview: "results/new.png",
                });
                emit("agent", {
                  kind: "ToolResult",
                  frame_id: fid,
                  name: "view_image",
                  ok: true,
                  content: "output verified",
                });
                emit("agent", {
                  kind: "Text",
                  frame_id: fid,
                  delta: "I inspected `old.csv` and created the requested output. See `notes/FIGURE_LEGEND.md`.",
                });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            // Interleaved commentary, reasoning, and tool calls exercise the
            // transcript's three-layer activity flow.
            if (String(arg("message") ?? "").includes("STEPMARKDOWN")) {
              return await new Promise<string>((resolve) => {
                setTimeout(() => {
                  emit("agent", { kind: "User", frame_id: fid, text: msg });
                  emit("agent", {
                    kind: "Text",
                    frame_id: fid,
                    delta: [
                      "### Live analysis",
                      "",
                      "**Significant result**",
                      "",
                      "- first finding",
                      "- second finding",
                      "",
                      "| Gene | Score |",
                      "| --- | ---: |",
                      "| ESR1 | 0.98 |",
                      "",
                      "`normalized_counts.csv`",
                    ].join("\n"),
                  });
                  setTimeout(() => {
                    emit("agent", { kind: "ToolCall", frame_id: fid, name: "shell", preview: "continue analysis" });
                    setTimeout(() => {
                      emit("agent", { kind: "ToolResult", frame_id: fid, name: "shell", ok: true, content: "done" });
                      emit("agent", { kind: "Done", frame_id: fid });
                      resolve(fid);
                    }, 5_000);
                  }, 1_000);
                }, 30);
              });
            }
            if (String(arg("message") ?? "").includes("STEPSDEMO")) {
              return await new Promise<string>((resolve) => {
                setTimeout(() => {
                  emit("agent", { kind: "User", frame_id: fid, text: msg });
                  emit("agent", { kind: "Text", frame_id: fid, delta: "I’ll inspect the count matrix header first." });
                  emit("agent", { kind: "Reasoning", frame_id: fid, delta: "Let me inspect the count matrix header first." });
                  emit("agent", { kind: "ToolCall", frame_id: fid, name: "shell", preview: "zcat counts.txt.gz | head" });
                  emit("agent", { kind: "ToolResult", frame_id: fid, name: "shell", ok: true, content: Array.from({ length: 8 }, (_, i) => `gene_${i}\t12\t8\t15`).join("\n") });
                  emit("agent", { kind: "Text", frame_id: fid, delta: "I’ll load the full matrix and summarize it." });
                  emit("agent", { kind: "Reasoning", frame_id: fid, delta: "Now load the full matrix and summarize." });
                  emit("agent", { kind: "ToolCall", frame_id: fid, name: "python", preview: "import pandas as pd\ndf = pd.read_csv('counts.txt.gz', sep='\\t')" });
                  emit("agent", { kind: "ToolResult", frame_id: fid, name: "python", ok: true, content: Array.from({ length: 18 }, (_, i) => `col_${i}: ok`).join("\n") });
                  emit("agent", { kind: "Text", frame_id: fid, delta: "I’ll save the reusable analysis script." });
                  emit("agent", { kind: "ToolCall", frame_id: fid, name: "write", preview: "/mock/root/deseq2.R" });
                  emit("agent", { kind: "ToolResult", frame_id: fid, name: "write", ok: true, content: "" });
                  emit("agent", { kind: "Text", frame_id: fid, delta: "The data is clean: 60,675 genes × 15 samples in a 2×2 factorial design." });
                  emit("agent", { kind: "Done", frame_id: fid });
                  resolve(fid);
                }, 30);
              });
            }
            if (String(arg("message") ?? "").includes("MDLIST")) {
              const md = [
                "FX细胞（FX cell）是一种常用于病毒学研究的人源细胞系，具有以下特点：",
                "",
                "- **来源**：从人胚肾细胞（HEK293）衍生",
                "- **应用**：广泛用于慢病毒载体包装和生产",
                "- **优势**：转染效率高，适合大规模病毒生产",
                "",
                "有什么我可以帮你的吗？",
              ].join("\n");
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: md });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("MDURL")) {
              const md = "这是百度的网址：https://www.baidu.com，测试成功了吗？";
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: md });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("MDLEADBAR")) {
              const md = [
                "**结论**：字号已经跟随设置。",
                "",
                "表格字号现在跟随外观设置，不再写死。",
              ].join("\n");
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: md });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("MDTABLE")) {
              const md = [
                "| Tissue | TPM |",
                "|---|---:|",
                "| Veg 0DAF | 2.62 |",
                "| Notch 0DAF | 1.81 |",
              ].join("\n");
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: md });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            if (String(arg("message") ?? "").includes("MDCODE")) {
              const md = [
                "缺少的是：",
                "",
                "```text",
                "CAF状态 → 免疫变化",
                "CAF状态 → 上皮变化",
                "```",
                "",
                "```python",
                "def immune_change(caf_status):",
                "    # 暗色代码注释",
                "    return \"免疫变化\" if caf_status else None",
                "```",
                "",
                "```diff",
                "-CAF状态 → 未知",
                "+CAF状态 → 免疫变化",
                "```",
              ].join("\n");
              setTimeout(() => {
                emit("agent", { kind: "User", frame_id: fid, text: msg });
                emit("agent", { kind: "Text", frame_id: fid, delta: md });
                emit("agent", { kind: "Done", frame_id: fid });
              }, 30);
              return fid;
            }
            setTimeout(() => {
              emit("agent", { kind: "User", frame_id: fid, text: msg });
              emit("agent", { kind: "ToolCall", frame_id: fid, name: "read", preview: "mock context" });
              emit("agent", { kind: "ToolResult", frame_id: fid, name: "read", ok: true, content: "ok" });
              emit("agent", { kind: "Text", frame_id: fid, delta: "Hello " });
              emit("agent", { kind: "Text", frame_id: fid, delta: "from mock wisp-science." });
              emit("agent", { kind: "Done", frame_id: fid });
            }, 50 + Number((window as any).__userEventDelayMs ?? 0));
            return fid;
          }
          case "open_external_url":
            if (arg("url")) window.open(String(arg("url")), "_blank", "noopener,noreferrer");
            return null;
          case "open_browser_extension_page":
            return { extension_path: "/mock/wisp/browser-extension", opened: false };
          case "browser_extension_status":
            return (window as any).__browserExtensionStatus ?? {
              connected: false,
              current_version: null,
              bundled_version: "0.3.1",
              current_protocol: 0,
              required_protocol: 2,
              update_required: false,
              automatic_reload_available: false,
              extension_path: "/mock/wisp/browser-extension",
              extension_path_verified: true,
              integrity_verified: true,
              error: null,
            };
          case "update_browser_extension":
            return (window as any).__browserExtensionUpdateResult ?? {
              outcome: "manual_reload_required",
              status: (window as any).__browserExtensionStatus,
              opened: true,
              error: null,
            };
          case "extension_connected":
            return Boolean((window as any).__extensionConnected);
          case "ui_heartbeat":
            return null;
          case "list_specialists":
            return mockSpecialists;
          case "save_specialist_cmd": {
            const spec = plain(arg("spec") ?? {});
            if (!spec.id) { spec.id = `sp${mockSpecialists.length}`; spec.builtin = false; }
            mockSpecialists = mockSpecialists.some((s) => s.id === spec.id)
              ? mockSpecialists.map((s) => (s.id === spec.id ? { ...s, ...spec, builtin: s.builtin, instructions: s.builtin ? s.instructions : spec.instructions } : s))
              : [...mockSpecialists, spec];
            return mockSpecialists;
          }
          case "test_reviewer_backend": {
            const reviewer = plain(arg("reviewer") ?? {});
            const config = reviewer.review_backend ?? { kind: "http_model", profile_id: reviewer.model_id ?? "" };
            if (config.kind === "acp_agent") {
              const profile = mockAcpAgents.find((agent) => agent.id === config.profile_id);
              if (!profile) throw new Error("The Reviewer ACP Agent profile no longer exists.");
              return {
                backend: "acp_agent",
                model: profile.label,
                status: "passed",
                summary: "The reported sample count matches the tool output.",
              };
            }
            const profile = mockModels.find((model) => model.id === config.profile_id)
              ?? mockModels.find((model) => model.active)
              ?? mockModels[0];
            return {
              backend: "http_model",
              model: profile?.model ?? profile?.label ?? "default",
              status: "passed",
              summary: "The reported sample count matches the tool output.",
            };
          }
          case "remove_specialist": {
            const id = arg("id");
            if (mockSpecialists.find((s) => s.id === id)?.builtin) throw new Error("Built-in specialists cannot be removed.");
            mockSpecialists = mockSpecialists.filter((s) => s.id !== id);
            return mockSpecialists;
          }
          case "set_session_specialist":
            sessionSpecialists[arg("frameId")] = arg("id");
            return null;
          case "get_session_specialist":
            return mockSpecialists.find((s) => s.id === sessionSpecialists[arg("frameId")]) ?? null;
          case "mcp_app_has_server_tools":
            return Boolean((window as any).__mcpAppLiveBridges);
          case "list_mcp_app_tools":
            if (!(window as any).__mcpAppLiveBridges) {
              throw new Error("stale-instance: the MCP App is no longer bound to a live MCP server");
            }
            return {
              tools: (window as any).__mcpAppTools ?? [{
                name: "figure_preview_exact",
                inputSchema: { type: "object" },
              }],
            };
          case "call_mcp_app_tool": {
            if (!(window as any).__mcpAppLiveBridges) {
              throw new Error("stale-instance: the MCP App is no longer bound to a live MCP server");
            }
            const name = String(arg("name") ?? "");
            const canned = (window as any).__mcpAppToolResults;
            if (canned && canned[name]) return canned[name];
            return {
              content: [{ type: "text", text: "exact preview" }],
              structuredContent: { preview: true, tool: name },
              isError: false,
            };
          }
          case "close_mcp_app":
            return true;
          default:
            return null;
        }
      },
    },
    event: {
      listen: async (event: string, cb: (e: { payload: unknown }) => void) => {
        listeners[event] = cb;
        return () => {
          listeners[event] = undefined;
        };
      },
    },
    window: {
      getCurrentWindow: () => ({
        listen: async (event: string, cb: (e: { payload: unknown }) => void) => {
          windowListeners[event] = cb;
          return () => {
            windowListeners[event] = undefined;
          };
        },
        startDragging: async () => {
          (window as any).__petDragStarted = true;
        },
        toggleMaximize: async () => {
          ((window as any).__skillInvokeLog ??= []).push({ cmd: "toggle-maximize" });
        },
        minimize: async () => {
          ((window as any).__skillInvokeLog ??= []).push({ cmd: "minimize" });
        },
        close: async () => {
          ((window as any).__skillInvokeLog ??= []).push({ cmd: "close" });
        },
        setTitle: async (title: string) => {
          document.title = String(title);
          ((window as any).__skillInvokeLog ??= []).push({ cmd: "set-title", args: { title } });
        },
      }),
    },
  };
}

// Expected assistant reply text for a message sent under `parallelMock`.
// Specs that must pin exact reply content derive it from these helpers so the
// `echo:` convention stays an internal detail of this mock. (`parallelMock`
// itself is serialized into the page by addInitScript, so it cannot call
// these; its inline template below must stay in sync.)
export function parallelReplyText(message: string): string {
  return `echo:${message}`;
}

export function parallelReplyTailText(message: string): string {
  return `${parallelReplyText(message)}:tail`;
}

// Variant for parallel-session tests: each `send_message` streams a reply
// quoting the message (see `parallelReplyText`) immediately but delays `Done`
// so the session stays "running" while the test starts a second conversation.
// `list_sessions` reports every session that received a user turn so the
// sidebar can list them.
export function parallelMock(): void {
  const params = new URLSearchParams(window.location.search);
  const listeners: Record<string, ((e: { payload: unknown }) => void) | undefined> = {};
  const windowListeners: Record<string, ((e: { payload: unknown }) => void) | undefined> = {};
  const emit = (event: string, payload: unknown) => {
    try {
      listeners[event]?.({ payload });
      windowListeners[event]?.({ payload });
    } catch { /* not registered yet */ }
  };
  const sessions: { id: string; title: string; ts: number; folder_id: string | null; stale_prompt?: boolean }[] = [];
  if (params.get("mockStaleRules") === "1") {
    sessions.push({ id: "stale-session", title: "Outdated rules chat", ts: 1999, folder_id: null, stale_prompt: true });
  }
  const folders: { id: string; name: string }[] = [];
  const queues: Record<string, Promise<void>> = {};

  const project = { id: "default", name: "wisp-science", root: "/mock/root", skill_count: 12, mcp_server_count: 8, memory_file_count: 2, has_api_key: true };

  (window as any).__TAURI__ = {
    core: {
      invoke: async (cmd: string, args: any) => {
        ((window as any).__sendInvokeLog ??= []).push({ cmd, args });
        const arg = (key: string) => args instanceof Map ? args.get(key) : args?.[key];
        switch (cmd) {
          case "list_demos": return [];
          case "load_demo": return { id: "x", title: "x", request: "x", response: "x" };
          case "copy_demo_to_project": return `copied-${String(arg("id") ?? "demo")}`;
          case "load_session": {
            const delay = Number((window as any).__parallelLoadDelayMs ?? 0);
            if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
            // Counts loads whose (possibly delayed) snapshot has been handed to
            // the UI, so tests can wait for the late snapshot instead of sleeping.
            (window as any).__parallelLoadsResolved =
              ((window as any).__parallelLoadsResolved ?? 0) + 1;
            return { items: [], next_before_seq: null, user_offset: 0 };
          }
          case "list_sessions_page": return {
            items: sessions.slice(),
            next_cursor: null,
            running_ids: sessions.filter((item: any) => item.running).map((item) => item.id),
          };
          case "latest_used_session":
            return sessions.find((session) => session.has_user_turn !== false)?.id ?? null;
          case "list_folders": return folders.slice();
          case "create_folder": {
            const folder = { id: `folder-${folders.length + 1}`, name: String(arg("name") ?? "") };
            folders.push(folder);
            return folder;
          }
          case "rename_folder": {
            const folder = folders.find((entry) => entry.id === arg("id"));
            if (folder) folder.name = String(arg("name") ?? folder.name);
            return null;
          }
          case "delete_folder": {
            const index = folders.findIndex((entry) => entry.id === arg("id"));
            if (index >= 0) folders.splice(index, 1);
            return null;
          }
          case "list_projects":
            return [
              { id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 },
              { id: "other", name: "Other project", workspace_dir: "/mock/other", session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 },
              { id: "archive", name: "Archive project", workspace_dir: "/mock/archive", session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 },
            ];
          case "list_recent_sessions": return sessions.map((s) => ({
            id: s.id, project_id: "default", title: s.title, ts: s.ts,
            status: "complete",
          }));
          case "pick_directory": return "/mock/root/new-project";
          case "pick_executable_file": return "/mock/picked/Rscript";
          case "open_project":
          case "create_project":
            return { id: "default", name: project.name, workspace_dir: project.root, session_count: 0, updated_at: 1, running_count: 0, needs_you_count: 0 };
          case "delete_project": return null;
          case "get_bootstrap_status": return {
            skills_loaded: 12,
            python_ok: true,
            python_initializing: false,
            mcp_catalog: 8,
            uv_ok: true,
            node_ok: true,
            npm_ok: true,
            sci_ok: true,
            pixi_ok: true,
            app_version: "0.29.0",
            os: "windows",
            arch: "x86_64",
            startup: "total=120ms store=90ms window_ready=600000ms",
            workspace: project.root,
            errors: [],
          };
          case "get_appearance_prefs":
            return {
              saved: false,
              theme: "system",
              light_palette: "paper",
              dark_palette: "charcoal",
              ui_font_size: 14,
              code_font_size: 12,
              ui_font_family: "",
              code_font_family: "",
              selection_popup_enabled: true,
              send_with_modifier: false,
              custom_css: "",
            };
          case "set_appearance_prefs":
            return arg("prefs") ?? null;
          case "get_settings": return {
            provider: "openai",
            api_url: "https://api.deepseek.com",
            model: "deepseek-v4-pro",
            label: "deepseek-v4-pro",
            has_api_key: true,
            locale: "en",
            auto_compact: true,
            follow_up_questions: true,
            resume_last_session: true,
            supports_vision: true,
            sync_backend: "relay",
            sync_relay_url: "https://relay.example.test",
            sync_folder: "",
            sync_relay_token: "",
            has_sync_relay_token: true,
          };
          case "set_session_service_tier": {
            const sessionId = String(arg("sessionId") ?? "");
            const tier = arg("serviceTier");
            if (tier === null || typeof tier === "undefined") {
              delete sessionServiceTiers[sessionId];
            } else {
              sessionServiceTiers[sessionId] = String(tier);
            }
            return null;
          }
          case "get_project_info": return project;
          case "generate_follow_up_questions": return [
            "Review the records that need manual correction",
            "Expand the search for underrepresented species",
            "Generate a literature landscape visualization",
          ];
          case "get_onboarding_state": return { show: false, has_api_key: true };
          case "get_capabilities": return { skills: [], mcp_servers: [], memory_files: [], project };
          case "list_approval_grants": return [];
          case "list_dir": return [];
          case "create_file":
          case "create_directory":
          case "rename_entry":
          case "delete_entry": return null;
          case "search_files": return [];
          case "search_artifacts": return [];
          case "read_file": return { path: "x", mime: "text/plain", text: "", base64: null };
          case "missing_files": return [];
          case "export_session": return "/mock/export.zip";
          case "export_session_trajectory":
            if ((window as any).__trajectoryExportCancel) return null;
            return "/mock/wisp-trajectory.html";
          case "import_session_archive": return {
            frame_id: "imported-frame", status: "imported",
            message_count: 3, artifact_count: 0, missing_artifacts: [],
          };
          case "upload_file": return { id: "a", name: "x", kind: "text/csv", path: "x", ts: 1 };
          case "upload_to_context": return [{
            sourcePath: "/mock/local/counts.csv",
            destinationPath: "/home/research/counts.csv",
            runId: "run-upload-1",
            status: "running",
          }];
          case "new_session": return `s-${Math.random().toString(36).slice(2)}`;
          case "start_scratch_chat": {
            scratchOpen = true;
            scratchSessionId = `scratch-${Math.random().toString(36).slice(2)}`;
            return { sessionId: scratchSessionId, projectId: "scratch:mock" };
          }
          case "close_scratch_chat": {
            scratchOpen = false;
            scratchSessionId = null;
            return null;
          }
          case "rename_session": {
            const session = sessions.find((entry) => entry.id === arg("id"));
            if (session) {
              session.title = String(arg("title") ?? session.title);
            } else {
              // Mirrors the store: naming a message-less draft makes it
              // listable before its first user turn (#888).
              sessions.unshift({
                id: String(arg("id")), title: String(arg("title") ?? ""),
                ts: Date.now(), folder_id: null,
                has_user_turn: false,
              });
            }
            return null;
          }
          case "delete_session": {
            const index = sessions.findIndex((entry) => entry.id === arg("id"));
            if (index >= 0) sessions.splice(index, 1);
            return null;
          }
          case "reload_project_rules": {
            const session = sessions.find((entry) => entry.id === String(arg("frameId")));
            if (session) session.stale_prompt = false;
            return true;
          }
          case "move_session": {
            const session = sessions.find((entry) => entry.id === arg("id"));
            if (session) session.folder_id = (arg("folderId") as string | null) ?? null;
            return null;
          }
          case "transfer_session_to_project": {
            if (arg("mode") === "move") {
              const index = sessions.findIndex((entry) => entry.id === arg("id"));
              if (index >= 0) sessions.splice(index, 1);
            }
            return `transferred-${String(arg("id"))}`;
          }
          case "stop_agent":
          case "rewind_session":
          case "revoke_approval_grant":
          case "revoke_all_approval_grants":
          case "confirm_response":
          case "dismiss_onboarding":
            return null;
          case "validate_settings": return "ok";
          case "check_for_updates":
            return {
              current_version: "0.9.0",
              latest_version: "0.9.0",
              update_available: false,
              release_url: "https://github.com/xuzhougeng/wisp-science/releases",
            };
          case "send_message": {
            const fid = (args && (args.sessionId ?? args.session_id)) || "t1";
            const msg = (args && args.message) || "";
            const run = async () => {
              if (!sessions.some((s) => s.id === fid)) {
                sessions.push({ id: fid, title: msg, ts: Date.now(), folder_id: null });
              }
              emit("agent", { kind: "User", frame_id: fid, text: msg });
              emit("agent", { kind: "Text", frame_id: fid, delta: `echo:${msg}` });
              if (msg === "alpha") {
                await new Promise((resolve) => setTimeout(resolve, 1200));
                emit("agent", { kind: "Text", frame_id: fid, delta: ":tail" });
                await new Promise((resolve) => setTimeout(resolve, 3800));
              } else if (msg.startsWith("actions-")) {
                await new Promise((resolve) => setTimeout(resolve, 50));
              } else {
                await new Promise((resolve) => setTimeout(resolve, 5000));
              }
              emit("agent", { kind: "Done", frame_id: fid });
            };
            const previous = queues[fid] ?? Promise.resolve();
            const current = previous.then(run, run);
            queues[fid] = current.catch(() => undefined);
            await current;
            return fid;
          }
          case "enqueue_turn": {
            // Queue (#433): chain the parked turn onto the same per-session
            // promise chain send_message uses, so it drains FIFO after the
            // running turn finishes — mirroring the backend driver. Returns
            // immediately (the real command is fast and non-blocking).
            const fid = (args && (args.sessionId ?? args.session_id)) || "t1";
            const msg = (args && args.message) || "";
            const run = async () => {
              emit("agent", { kind: "User", frame_id: fid, text: msg });
              emit("agent", { kind: "Text", frame_id: fid, delta: `echo:${msg}` });
              await new Promise((resolve) => setTimeout(resolve, 50));
              emit("agent", { kind: "Done", frame_id: fid });
            };
            const previous = queues[fid] ?? Promise.resolve();
            const current = previous.then(run, run);
            queues[fid] = current.catch(() => undefined);
            return null;
          }
          case "queued_turn_action": return null;
          case "open_external_url":
            if (arg("url")) window.open(String(arg("url")), "_blank", "noopener,noreferrer");
            return null;
          case "open_browser_extension_page":
            return { extension_path: "/mock/wisp/browser-extension", opened: false };
          case "browser_extension_status":
            return (window as any).__browserExtensionStatus ?? {
              connected: false,
              current_version: null,
              bundled_version: "0.3.1",
              current_protocol: 0,
              required_protocol: 2,
              update_required: false,
              automatic_reload_available: false,
              extension_path: "/mock/wisp/browser-extension",
              extension_path_verified: true,
              integrity_verified: true,
              error: null,
            };
          case "update_browser_extension":
            return (window as any).__browserExtensionUpdateResult ?? {
              outcome: "manual_reload_required",
              status: (window as any).__browserExtensionStatus,
              opened: true,
              error: null,
            };
          case "extension_connected":
            return Boolean((window as any).__extensionConnected);
          case "ui_heartbeat":
            return null;
          default: return null;
        }
      },
    },
    event: {
      listen: async (event: string, cb: (e: { payload: unknown }) => void) => {
        listeners[event] = cb;
        return () => { listeners[event] = undefined; };
      },
    },
    window: {
      getCurrentWindow: () => ({
        listen: async (event: string, cb: (e: { payload: unknown }) => void) => {
          windowListeners[event] = cb;
          return () => { windowListeners[event] = undefined; };
        },
        setTitle: async (title: string) => {
          document.title = String(title);
          ((window as any).__skillInvokeLog ??= []).push({ cmd: "set-title", args: { title } });
        },
      }),
    },
  };
}
