#!/usr/bin/env Rscript

# Persistent R execution over Wisp's versioned JSON-lines protocol.
# stdout is reserved for protocol frames; user output is captured in results.

MAX_OUTPUT_SIZE <- 1024L * 1024L
MAX_OBJECTS <- 200L
MAX_NAME_SIZE <- 256L
MAX_META_SIZE <- 160L
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

object_summary <- function(value) {
  dimensions <- dim(value)
  if (!is.null(dimensions) && length(dimensions) > 0L) {
    return(paste(dimensions, collapse = " × "))
  }
  if (is.atomic(value) && length(value) == 1L && !is.object(value)) {
    rendered <- as.character(value)
    if (nchar(rendered, type = "bytes") <= 80L) {
      return(rendered)
    }
  }
  if (is.environment(value)) {
    return(paste0(length(ls(envir = value, all.names = TRUE)), " bindings"))
  }
  if (is.list(value)) {
    return(paste0(length(value), " items"))
  }
  if (is.atomic(value) && length(value) > 1L) {
    return(paste0(length(value), " values"))
  }
  ""
}

inspect_runtime <- function() {
  names <- sort(ls(envir = runtime_env, all.names = FALSE))
  visible_names <- head(names, MAX_OBJECTS)
  objects <- lapply(visible_names, function(name) {
    tryCatch({
      value <- get(name, envir = runtime_env, inherits = FALSE)
      classes <- class(value)
      size <- if (is.atomic(value) || is.data.frame(value)) {
        tryCatch(as.numeric(object.size(value)), error = function(condition) NULL)
      } else {
        NULL
      }
      list(
        name = substr(name, 1L, MAX_NAME_SIZE),
        typeName = substr(
          if (length(classes) > 0L) classes[[1L]] else typeof(value),
          1L,
          MAX_META_SIZE
        ),
        summary = substr(object_summary(value), 1L, MAX_META_SIZE),
        sizeBytes = size
      )
    }, error = function(condition) {
      list(
        name = substr(name, 1L, MAX_NAME_SIZE),
        typeName = "unavailable",
        summary = substr(conditionMessage(condition), 1L, MAX_META_SIZE),
        sizeBytes = NULL
      )
    })
  })
  list(objects = objects, totalCount = length(names))
}

# Plotting must never open a desktop window. Users can still call png(), pdf(),
# ggsave(), or another explicit file device for context-local artifacts. The
# implicit device keeps its display list enabled so each cell's current plot
# can be snapshotted for the workbench plots pane without closing the device —
# incremental drawing (plot() now, lines() in a later cell) keeps working.
plot_state <- new.env(parent = emptyenv())
plot_state$devices <- integer()
plot_state$last_snapshot <- NULL

wisp_default_device <- function(...) {
  grDevices::pdf(file = NULL)
  grDevices::dev.control(displaylist = "enable")
  plot_state$devices <- c(plot_state$devices, grDevices::dev.cur())
}
options(device = wisp_default_device)

# Base64 PNG of the implicit device's current plot, or character() when there
# is nothing new. Only devices opened by wisp_default_device are snapshotted:
# a user's own png()/pdf() device already writes their chosen file. Re-emitting
# an unchanged display list is suppressed so a plot-free cell after a plotting
# cell reports no plots.
collect_plots <- function() {
  current <- grDevices::dev.cur()
  if (current == 1L || !(current %in% plot_state$devices)) {
    return(character())
  }
  snapshot <- tryCatch(grDevices::recordPlot(), error = function(condition) NULL)
  if (is.null(snapshot) || is.null(snapshot[[1L]])) {
    return(character())
  }
  file <- tempfile(fileext = ".png")
  on.exit(unlink(file), add = TRUE)
  rendered <- tryCatch(
    {
      grDevices::png(filename = file, width = 896L, height = 632L, res = 108L)
      replay_device <- grDevices::dev.cur()
      grDevices::replayPlot(snapshot)
      grDevices::dev.off(replay_device)
      file.exists(file) && file.size(file) > 0L
    },
    error = function(condition) {
      if (grDevices::dev.cur() != current) {
        try(grDevices::dev.off(), silent = TRUE)
      }
      FALSE
    }
  )
  if (current %in% grDevices::dev.list()) {
    grDevices::dev.set(current)
  }
  if (!isTRUE(rendered)) {
    return(character())
  }
  encoded <- jsonlite::base64_enc(readBin(file, what = "raw", n = file.size(file)))
  encoded <- gsub("[\r\n]", "", encoded)
  if (identical(encoded, plot_state$last_snapshot)) {
    return(character())
  }
  plot_state$last_snapshot <- encoded
  encoded
}

evaluate_cell <- function(code, source_name = NULL) {
  source_file <- if (!is.null(source_name)) {
    srcfilecopy(source_name, strsplit(code, "\n", fixed = TRUE)[[1L]])
  } else {
    NULL
  }
  expressions <- parse(text = code, srcfile = source_file, keep.source = TRUE)
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

execute_cell <- function(code, source_name = NULL) {
  diagnostics <- character()
  error_text <- NULL
  started <- proc.time()

  output <- capture.output(
    tryCatch(
      withCallingHandlers(
        evaluate_cell(code, source_name),
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
    plots = collect_plots(),
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
  if (is.null(request)) {
    next
  }

  request_id <- if (is.character(request$id) && length(request$id) == 1L) {
    request$id
  } else {
    "unknown"
  }
  if (identical(request$type, "inspect")) {
    inspection <- inspect_runtime()
    emit_frame(list(
      type = "objects",
      id = request_id,
      objects = inspection$objects,
      totalCount = inspection$totalCount
    ))
    next
  }
  if (!identical(request$type, "execute")) {
    next
  }

  code <- if (is.character(request$code) && length(request$code) == 1L) {
    request$code
  } else {
    ""
  }
  required_objects <- if (is.null(request$required_objects)) {
    character()
  } else if (
    is.list(request$required_objects) &&
      all(vapply(
        request$required_objects,
        function(name) is.character(name) && length(name) == 1L && nzchar(name),
        logical(1)
      ))
  ) {
    unlist(request$required_objects, use.names = FALSE)
  } else {
    NULL
  }
  if (is.null(required_objects)) {
    emit_frame(list(
      type = "result",
      id = request_id,
      stdout = "",
      stderr = "",
      error = "required_objects must be an array of non-empty strings",
      interrupted = FALSE,
      usage = list(wall_s = 0, cpu_s = 0, rss_kb = 0)
    ))
    next
  }
  missing_objects <- required_objects[!vapply(
    required_objects,
    exists,
    logical(1),
    envir = runtime_env,
    inherits = FALSE
  )]
  if (length(missing_objects) > 0L) {
    emit_frame(list(
      type = "result",
      id = request_id,
      stdout = "",
      stderr = "",
      error = paste0(
        "required runtime objects are missing: ",
        paste(missing_objects, collapse = ", ")
      ),
      interrupted = FALSE,
      usage = list(wall_s = 0, cpu_s = 0, rss_kb = 0)
    ))
    next
  }
  source_name <- if (
    is.character(request$source_name) && length(request$source_name) == 1L &&
      nzchar(request$source_name)
  ) {
    request$source_name
  } else {
    NULL
  }
  result <- execute_cell(code, source_name)
  frame <- list(
    type = "result",
    id = request_id,
    stdout = result$stdout,
    stderr = result$stderr,
    error = result$error,
    interrupted = FALSE,
    usage = result$usage
  )
  if (length(result$plots) > 0L) {
    # I() keeps a single snapshot serialized as a JSON array, not a string.
    frame$plots <- I(result$plots)
  }
  emit_frame(frame)
}
