#!/usr/bin/env Rscript

# Persistent R execution over Wisp's versioned JSON-lines protocol.
# stdout is reserved for protocol frames; user output is captured in results.

MAX_OUTPUT_SIZE <- 1024L * 1024L
protocol_in <- file("stdin", open = "r", encoding = "UTF-8")
protocol_out <- stdout()

if (!requireNamespace("jsonlite", quietly = TRUE)) {
  cat(
    '{"type":"startup_error","error":"R package \'jsonlite\' is required; install it in the selected R environment"}\n',
    file = protocol_out
  )
  flush(protocol_out)
  quit(save = "no", status = 78L, runLast = FALSE)
}

emit_frame <- function(frame) {
  cat(
    jsonlite::toJSON(frame, auto_unbox = TRUE, null = "null", digits = NA),
    "\n",
    sep = "",
    file = protocol_out
  )
  flush(protocol_out)
}

truncate_text <- function(text) {
  text <- paste(text, collapse = "\n")
  size <- nchar(text, type = "bytes")
  if (is.na(size) || size <= MAX_OUTPUT_SIZE) {
    return(text)
  }

  bytes <- charToRaw(enc2utf8(text))
  head <- rawToChar(bytes[seq_len(MAX_OUTPUT_SIZE)])
  head <- iconv(head, from = "UTF-8", to = "UTF-8", sub = "")
  if (is.na(head)) {
    head <- ""
  }
  paste0(head, "\n... (truncated, ", size - MAX_OUTPUT_SIZE, " bytes omitted)")
}

runtime_env <- new.env(parent = globalenv())

# Plotting must never open a desktop window. Users can still call png(), pdf(),
# ggsave(), or another explicit file device for context-local artifacts.
options(device = function(...) grDevices::pdf(file = NULL))

evaluate_cell <- function(code) {
  expressions <- parse(text = code, keep.source = TRUE)
  if (length(expressions) == 0L) {
    return(invisible(NULL))
  }

  for (index in seq_along(expressions)) {
    value <- withVisible(eval(expressions[[index]], envir = runtime_env))
    if (index == length(expressions) && isTRUE(value$visible)) {
      print(value$value)
    }
  }
  invisible(NULL)
}

execute_cell <- function(code) {
  diagnostics <- character()
  error_text <- NULL
  started <- proc.time()

  output <- capture.output(
    tryCatch(
      withCallingHandlers(
        evaluate_cell(code),
        warning = function(condition) {
          diagnostics <<- c(
            diagnostics,
            paste0("Warning: ", conditionMessage(condition))
          )
          invokeRestart("muffleWarning")
        },
        message = function(condition) {
          diagnostics <<- c(diagnostics, conditionMessage(condition))
          invokeRestart("muffleMessage")
        }
      ),
      error = function(condition) {
        call <- conditionCall(condition)
        calls <- sys.calls()
        call_text <- if (is.null(call)) {
          ""
        } else {
          paste0("\nCall: ", paste(deparse(call), collapse = " "))
        }
        trace_text <- if (length(calls) == 0L) {
          ""
        } else {
          rendered <- vapply(
            calls,
            function(item) paste(deparse(item), collapse = " "),
            character(1)
          )
          paste0("\nTraceback:\n", paste(rendered, collapse = "\n"))
        }
        error_text <<- paste0(conditionMessage(condition), call_text, trace_text)
        invisible(NULL)
      }
    ),
    type = "output"
  )

  elapsed <- proc.time() - started
  list(
    stdout = truncate_text(output),
    stderr = truncate_text(diagnostics),
    error = error_text,
    usage = list(
      wall_s = unname(elapsed[["elapsed"]]),
      cpu_s = unname(elapsed[["user.self"]] + elapsed[["sys.self"]]),
      rss_kb = 0L
    )
  )
}

emit_frame(list(
  type = "ready",
  protocol = 1L,
  language = "r",
  pid = Sys.getpid(),
  version = paste(R.version$major, R.version$minor, sep = ".")
))

repeat {
  line <- readLines(protocol_in, n = 1L, warn = FALSE)
  if (length(line) == 0L) {
    break
  }
  if (!nzchar(trimws(line))) {
    next
  }

  request <- tryCatch(
    jsonlite::fromJSON(line, simplifyVector = FALSE),
    error = function(condition) NULL
  )
  if (is.null(request) || !identical(request$type, "execute")) {
    next
  }

  request_id <- if (is.character(request$id) && length(request$id) == 1L) {
    request$id
  } else {
    "unknown"
  }
  code <- if (is.character(request$code) && length(request$code) == 1L) {
    request$code
  } else {
    ""
  }
  result <- execute_cell(code)
  emit_frame(list(
    type = "result",
    id = request_id,
    stdout = result$stdout,
    stderr = result$stderr,
    error = result$error,
    interrupted = FALSE,
    usage = result$usage
  ))
}
