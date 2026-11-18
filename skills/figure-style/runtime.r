# figure-style R sidecar: render-then-inspect QA for ggplot2 / grid figures.
#
# Loaded with source(..., local = TRUE) into the persistent R runtime. Loading
# only defines functions: it attaches no package, opens no device, and changes
# no option. ggplot2, patchwork, ragg and png are used through `::` when a
# helper runs and only if that helper needs them.
#
# The geometry checks walk the plot's grid grob tree the way grid draws it
# (makeContext -> push viewport -> makeContent -> children) on an off-screen
# device of the saved size, so text boxes are measured with the same fonts and
# viewport transforms the saved file uses.

# ---------------------------------------------------------------- internals

.wfs_size_in <- function(width, height, units, dpi) {
  factor <- switch(units, mm = 1 / 25.4, cm = 1 / 2.54, `in` = 1, px = 1 / dpi,
    stop("units must be one of mm, cm, in, px", call. = FALSE))
  c(width, height) * factor
}

# The device ggsave() uses for .png (ragg when installed), so measurement,
# crops, and the delivered file share one font stack.
.wfs_open_png <- function(file, w_in, h_in, dpi, bg) {
  if (requireNamespace("ragg", quietly = TRUE)) {
    ragg::agg_png(file, width = w_in, height = h_in, units = "in", res = dpi,
      background = bg)
  } else {
    grDevices::png(file, width = w_in, height = h_in, units = "in", res = dpi,
      bg = bg)
  }
  grDevices::dev.cur()
}

# Open an off-screen device, run `fun`, and always close it again, restoring
# whichever device was current before (the runtime's plot pane device).
.wfs_with_device <- function(w_in, h_in, dpi, fun, file = NULL, bg = "white") {
  previous <- grDevices::dev.cur()
  scratch <- is.null(file)
  if (scratch) file <- tempfile(fileext = ".png")
  device <- .wfs_open_png(file, w_in, h_in, dpi, bg)
  on.exit({
    if (device %in% grDevices::dev.list()) grDevices::dev.off(device)
    if (previous > 1L && previous %in% grDevices::dev.list()) grDevices::dev.set(previous)
    if (scratch) unlink(file)
  }, add = TRUE)
  grid::grid.newpage()
  fun()
}

.wfs_grob <- function(plot) {
  # patchwork objects also inherit "ggplot", so test them first.
  if (inherits(plot, "patchwork")) return(patchwork::patchworkGrob(plot))
  if (inherits(plot, c("ggplot", "ggplot2::ggplot"))) return(ggplot2::ggplotGrob(plot))
  if (inherits(plot, "grob")) return(plot)
  stop(paste(
    "text_collisions()/panel_crops() need a ggplot, patchwork, or grid grob.",
    "For base graphics use check_render() and inspect the file with view_image."
  ), call. = FALSE)
}

.wfs_depth <- function() {
  path <- grid::current.vpPath()
  if (is.null(path)) 0L else length(strsplit(as.character(path), "::", fixed = TRUE)[[1L]])
}

.wfs_push <- function(vp) {
  if (inherits(vp, "vpPath")) grid::downViewport(vp) else grid::pushViewport(vp)
}

# Climb back to `depth`. upViewport(0) means "to ROOT", so never pass it 0.
.wfs_up_to <- function(depth) {
  levels <- .wfs_depth() - depth
  if (levels > 0L) grid::upViewport(levels)
}

# Graphical parameters inherited down the grob tree; cex compounds.
.wfs_merge_gp <- function(parent, child) {
  if (is.null(child)) return(parent)
  child <- unclass(child)
  for (name in names(child)) {
    if (name == "cex" && !is.null(parent$cex)) {
      parent$cex <- parent$cex * child$cex
    } else {
      parent[[name]] <- child[[name]]
    }
  }
  parent
}

.wfs_at <- function(values, i) values[[(i - 1L) %% length(values) + 1L]]

.wfs_gp_at <- function(gp, i) {
  keep <- intersect(names(gp), c("fontsize", "cex", "fontfamily", "font", "lineheight"))
  do.call(grid::gpar, lapply(gp[keep], .wfs_at, i = i))
}

.wfs_invisible <- function(gp, i) {
  col <- if (is.null(gp$col)) NULL else .wfs_at(gp$col, i)
  alpha <- if (is.null(gp$alpha)) 1 else .wfs_at(gp$alpha, i)
  (!is.null(col) && (is.na(col) || identical(col, "transparent"))) ||
    (!is.na(alpha) && alpha <= 0)
}

.wfs_just <- function(just, hjust, vjust, i) {
  h <- 0.5
  v <- 0.5
  if (is.numeric(just)) {
    just <- rep_len(just, 2L)
    h <- just[[1L]]
    v <- just[[2L]]
  } else if (is.character(just) && length(just)) {
    hmap <- c(left = 0, right = 1, centre = 0.5, center = 0.5)
    vmap <- c(bottom = 0, top = 1, centre = 0.5, center = 0.5)
    if (length(just) == 1L) {
      if (just %in% names(vmap) && !just %in% c("centre", "center")) {
        v <- vmap[[just]]
      } else if (just %in% names(hmap)) {
        h <- hmap[[just]]
      }
    } else {
      if (just[[1L]] %in% names(hmap)) h <- hmap[[just[[1L]]]]
      if (just[[2L]] %in% names(vmap)) v <- vmap[[just[[2L]]]]
    }
  }
  if (length(hjust)) h <- .wfs_at(hjust, i)
  if (length(vjust)) v <- .wfs_at(vjust, i)
  c(h, v)
}

# A text grob's labels as an indexable vector: one plotmath call is one label.
.wfs_labels <- function(label) {
  if (is.factor(label)) return(as.character(label))
  if (is.language(label) && !is.expression(label)) return(as.expression(list(label)))
  label
}

.wfs_label_text <- function(label) {
  if (is.language(label) || is.expression(label)) {
    paste(deparse(if (is.expression(label)) label[[1L]] else label), collapse = "")
  } else {
    as.character(label)
  }
}

# Rotated text rectangle as 4 corners (device inches, bottom-left origin).
.wfs_text_corners <- function(text, labels, gp, i) {
  label <- labels[i]
  shown <- .wfs_label_text(label)
  if (!length(shown) || is.na(shown) || !nzchar(trimws(shown))) return(NULL)
  if (.wfs_invisible(gp, i)) return(NULL)
  x <- text$x[(i - 1L) %% length(text$x) + 1L]
  y <- text$y[(i - 1L) %% length(text$y) + 1L]
  anchor <- grid::deviceLoc(x, y, valueOnly = TRUE)
  probe <- grid::textGrob(label, gp = .wfs_gp_at(gp, i))
  w <- grid::convertWidth(grid::grobWidth(probe), "inches", valueOnly = TRUE)
  h <- grid::convertHeight(grid::grobHeight(probe), "inches", valueOnly = TRUE)
  just <- .wfs_just(text$just, text$hjust, text$vjust, i)
  rot <- if (length(text$rot)) .wfs_at(text$rot, i) else 0
  theta <- rot * pi / 180
  dx <- c(-just[[1L]], 1 - just[[1L]], 1 - just[[1L]], -just[[1L]]) * w
  dy <- c(-just[[2L]], -just[[2L]], 1 - just[[2L]], 1 - just[[2L]]) * h
  list(
    label = shown,
    x = anchor$x + dx * cos(theta) - dy * sin(theta),
    y = anchor$y + dx * sin(theta) + dy * cos(theta)
  )
}

# Separating-axis test for two convex quadrilaterals. Boxes must overlap by
# more than `eps` inches along every axis; merely touching does not count.
.wfs_quads_overlap <- function(a, b, eps) {
  for (quad in list(a, b)) {
    for (k in 1:2) {
      ex <- quad$x[[k + 1L]] - quad$x[[k]]
      ey <- quad$y[[k + 1L]] - quad$y[[k]]
      len <- sqrt(ex^2 + ey^2)
      if (len == 0) next
      nx <- -ey / len
      ny <- ex / len
      pa <- a$x * nx + a$y * ny
      pb <- b$x * nx + b$y * ny
      if (min(max(pa), max(pb)) - max(min(pa), min(pb)) <= eps) return(FALSE)
    }
  }
  TRUE
}

# Walk `grob` exactly as grid draws it, collecting every text label's box and
# every gtable cell's rectangle. Must run with an open device.
.wfs_scan <- function(grob) {
  texts <- list()
  cells <- list()
  table_id <- 0L

  walk <- function(g, gp, path, cell) {
    g <- grid::makeContext(g)
    before <- .wfs_depth()
    pushed <- !is.null(g$vp)
    if (pushed) .wfs_push(g$vp)
    if (pushed && !is.null(cell)) {
      lo <- grid::deviceLoc(grid::unit(0, "npc"), grid::unit(0, "npc"), valueOnly = TRUE)
      hi <- grid::deviceLoc(grid::unit(1, "npc"), grid::unit(1, "npc"), valueOnly = TRUE)
      cells[[length(cells) + 1L]] <<- c(cell, list(x0 = lo$x, y0 = lo$y, x1 = hi$x, y1 = hi$y))
    }
    gp <- .wfs_merge_gp(gp, g$gp)
    g <- grid::makeContent(g)
    if (inherits(g, "gTree")) {
      if (!is.null(g$childrenvp)) {
        inner <- .wfs_depth()
        .wfs_push(g$childrenvp)
        .wfs_up_to(inner)
      }
      layout <- if (inherits(g, "gtable")) g$layout else NULL
      if (!is.null(layout)) {
        table_id <<- table_id + 1L
        this_table <- table_id
      }
      for (name in g$childrenOrder) {
        child <- g$children[[name]]
        child_cell <- NULL
        if (!is.null(layout)) child_cell <- .wfs_cell(layout, child, this_table, path)
        walk(child, gp, c(path, name), child_cell)
      }
    } else if (inherits(g, "text") && length(g$label)) {
      labels <- .wfs_labels(g$label)
      kept <- list()
      for (i in seq_along(labels)) {
        quad <- .wfs_text_corners(g, labels, gp, i)
        if (is.null(quad)) next
        # check.overlap = TRUE makes grid drop a label that would overlap an
        # earlier one from the same grob, so it is never drawn.
        if (isTRUE(g$check.overlap) &&
            any(vapply(kept, .wfs_quads_overlap, logical(1), b = quad, eps = 0))) next
        kept[[length(kept) + 1L]] <- quad
        quad$path <- paste(path, collapse = "::")
        texts[[length(texts) + 1L]] <<- quad
      }
    }
    if (pushed) .wfs_up_to(before)
  }

  walk(grob, list(), character(), NULL)
  list(texts = texts, cells = cells)
}

# gtable wraps each child in a viewport named "<name>.<t>-<r>-<b>-<l>".
.wfs_cell <- function(layout, child, table, path) {
  vp_name <- if (inherits(child$vp, "viewport")) child$vp$name else NULL
  if (is.null(vp_name)) return(NULL)
  name <- sub("\\.[0-9]+-[0-9]+-[0-9]+-[0-9]+$", "", vp_name)
  rows <- which(layout$name == name)
  if (!length(rows)) return(NULL)
  if (length(rows) > 1L) {
    nums <- as.integer(strsplit(sub("^.*\\.", "", vp_name), "-", fixed = TRUE)[[1L]])
    exact <- rows[vapply(rows, function(r) {
      identical(as.integer(c(layout$t[r], layout$r[r], layout$b[r], layout$l[r])), nums)
    }, logical(1))]
    if (length(exact)) rows <- exact
  }
  r <- rows[[1L]]
  list(table = table, name = name, t = layout$t[r], l = layout$l[r],
    b = layout$b[r], r = layout$r[r], path = paste(path, collapse = "::"))
}

.wfs_spans_overlap <- function(a0, a1, b0, b1) a0 <= b1 && b0 <= a1

# ---------------------------------------------------------------- public helpers

check_render <- function(path) {
  # A "successful" R call with a missing, zero-byte, or blank file is a
  # failed render. Blank detection needs the png package and a .png file;
  # otherwise only existence and size are proven.
  if (!file.exists(path)) stop(sprintf("render failed: %s was not written", path), call. = FALSE)
  bytes <- file.info(path)$size
  if (is.na(bytes) || bytes == 0) stop(sprintf("render failed: %s is empty (0 bytes)", path), call. = FALSE)
  blank_checked <- FALSE
  if (grepl("\\.png$", path, ignore.case = TRUE) && requireNamespace("png", quietly = TRUE)) {
    image <- png::readPNG(path)
    channels <- if (length(dim(image)) == 2L) image else
      image[, , seq_len(min(3L, dim(image)[[3L]])), drop = FALSE]
    if (max(channels) - min(channels) < 1 / 255) {
      stop(sprintf("render failed: %s is a single flat colour (blank plot)", path), call. = FALSE)
    }
    blank_checked <- TRUE
  }
  invisible(list(path = path, bytes = bytes, blank_checked = blank_checked))
}

text_collisions <- function(plot, width, height, units = c("mm", "cm", "in", "px"), dpi = 300) {
  # Overlapping visible text pairs and text extending past the figure edge,
  # measured at the saved size. Returns list(overlaps, outside, n_labels).
  units <- match.arg(units)
  size <- .wfs_size_in(width, height, units, dpi)
  grob <- .wfs_grob(plot)
  texts <- .wfs_with_device(size[[1L]], size[[2L]], dpi, function() .wfs_scan(grob)$texts)
  eps <- 0.5 / 72  # half a point
  n <- length(texts)
  overlaps <- data.frame(a = character(), b = character(), a_path = character(),
    b_path = character(), stringsAsFactors = FALSE)
  outside <- data.frame(label = character(), path = character(), stringsAsFactors = FALSE)
  if (n) {
    x0 <- vapply(texts, function(t) min(t$x), numeric(1))
    x1 <- vapply(texts, function(t) max(t$x), numeric(1))
    y0 <- vapply(texts, function(t) min(t$y), numeric(1))
    y1 <- vapply(texts, function(t) max(t$y), numeric(1))
    for (i in seq_len(n - 1L)) {
      j <- (i + 1L):n
      j <- j[x0[j] < x1[i] - eps & x0[i] < x1[j] - eps & y0[j] < y1[i] - eps & y0[i] < y1[j] - eps]
      for (k in j) {
        if (.wfs_quads_overlap(texts[[i]], texts[[k]], eps)) {
          overlaps[nrow(overlaps) + 1L, ] <- list(texts[[i]]$label, texts[[k]]$label,
            texts[[i]]$path, texts[[k]]$path)
        }
      }
    }
    past <- which(x0 < -eps | y0 < -eps | x1 > size[[1L]] + eps | y1 > size[[2L]] + eps)
    for (i in past) outside[nrow(outside) + 1L, ] <- list(texts[[i]]$label, texts[[i]]$path)
  }
  list(overlaps = overlaps, outside = outside, n_labels = n)
}

panel_crops <- function(plot, width, height, units = c("mm", "cm", "in", "px"), dpi = 300,
                        pad_in = 0.08) {
  # One crop box per innermost plotting panel, unioned with the axis and
  # strip cells directly beside it. Boxes are inches from the figure's
  # bottom-left corner: data.frame(key, x0, y0, x1, y1).
  units <- match.arg(units)
  size <- .wfs_size_in(width, height, units, dpi)
  grob <- .wfs_grob(plot)
  cells <- .wfs_with_device(size[[1L]], size[[2L]], dpi, function() .wfs_scan(grob)$cells)
  empty <- data.frame(key = character(), x0 = numeric(), y0 = numeric(),
    x1 = numeric(), y1 = numeric(), stringsAsFactors = FALSE)
  panels <- Filter(function(cell) grepl("^panel", cell$name) &&
    cell$x1 > cell$x0 && cell$y1 > cell$y0, cells)
  # A patchwork or nested table can hold a panel cell that contains other
  # panels; crop the innermost ones.
  contains <- function(outer, inner) !identical(outer, inner) &&
    outer$x0 <= inner$x0 && outer$y0 <= inner$y0 && outer$x1 >= inner$x1 && outer$y1 >= inner$y1
  panels <- Filter(function(p) !any(vapply(panels, contains, logical(1), outer = p)), panels)
  if (!length(panels)) return(empty)
  out <- empty
  for (p in panels) {
    box <- c(p$x0, p$y0, p$x1, p$y1)
    for (cell in cells) {
      if (cell$table != p$table || !grepl("^(axis|strip)-[lrtb]", cell$name)) next
      beside_rows <- .wfs_spans_overlap(cell$t, cell$b, p$t, p$b) &&
        (cell$r == p$l - 1L || cell$l == p$r + 1L)
      beside_cols <- .wfs_spans_overlap(cell$l, cell$r, p$l, p$r) &&
        (cell$b == p$t - 1L || cell$t == p$b + 1L)
      if (beside_rows || beside_cols) {
        box <- c(min(box[[1L]], cell$x0), min(box[[2L]], cell$y0),
          max(box[[3L]], cell$x1), max(box[[4L]], cell$y1))
      }
    }
    box <- c(max(box[[1L]] - pad_in, 0), max(box[[2L]] - pad_in, 0),
      min(box[[3L]] + pad_in, size[[1L]]), min(box[[4L]] + pad_in, size[[2L]]))
    out[nrow(out) + 1L, ] <- list(as.character(nrow(out) + 1L), box[[1L]], box[[2L]],
      box[[3L]], box[[4L]])
  }
  out
}

save_panel_crops <- function(plot, boxes, width, height, units = c("mm", "cm", "in", "px"),
                             dpi = 300, bg = "white", out_dir = ".cache/figure-style") {
  # Render each crop box to out_dir and return the paths, named by key.
  # Crops are throwaway inspection files: out_dir is emptied on every call and
  # must never be the figures/output directory.
  units <- match.arg(units)
  size <- .wfs_size_in(width, height, units, dpi)
  grob <- .wfs_grob(plot)
  unlink(out_dir, recursive = TRUE, force = TRUE)
  dir.create(out_dir, recursive = TRUE, showWarnings = FALSE)
  paths <- character()
  for (row in seq_len(nrow(boxes))) {
    box <- boxes[row, ]
    w <- box$x1 - box$x0
    h <- box$y1 - box$y0
    if (w <= 0 || h <= 0) next
    path <- file.path(out_dir, paste0(box$key, ".png"))
    # Draw the whole figure at its saved size, shifted so this box fills the
    # device; text keeps its absolute point size, so the crop matches the file.
    .wfs_with_device(w, h, dpi, file = path, bg = bg, fun = function() {
      grid::pushViewport(grid::viewport(
        x = grid::unit(-box$x0, "inches"), y = grid::unit(-box$y0, "inches"),
        width = grid::unit(size[[1L]], "inches"), height = grid::unit(size[[2L]], "inches"),
        just = c("left", "bottom")
      ))
      grid::grid.draw(grob)
      grid::popViewport()
    })
    paths[[box$key]] <- path
  }
  paths
}

save_checked_figure <- function(plot, filename, width, height,
                                units = c("mm", "cm", "in", "px"), dpi = 300,
                                bg = "white", crops = TRUE,
                                out_dir = ".cache/figure-style") {
  # Save with ggsave(), then prove the render: the file exists, is non-empty
  # and not blank, no visible text overlaps other text or runs past the
  # figure edge. Any failure stops with the offending labels. On success it
  # writes per-panel QA crops to out_dir and returns list(path, crops); inspect
  # the figure and every crop with view_image before presenting it.
  units <- match.arg(units)
  size <- .wfs_size_in(width, height, units, dpi)
  dir.create(dirname(filename), recursive = TRUE, showWarnings = FALSE)
  ggplot2::ggsave(filename, plot = plot, width = size[[1L]], height = size[[2L]],
    units = "in", dpi = dpi, bg = bg)
  check_render(filename)
  scan <- text_collisions(plot, size[[1L]], size[[2L]], units = "in", dpi = dpi)
  problems <- c(
    sprintf("\"%s\" overlaps \"%s\"", scan$overlaps$a, scan$overlaps$b),
    sprintf("\"%s\" extends past the figure edge", scan$outside$label)
  )
  if (length(problems)) {
    shown <- utils::head(problems, 12L)
    if (length(problems) > length(shown)) {
      shown <- c(shown, sprintf("... and %d more", length(problems) - length(shown)))
    }
    stop(paste(c(sprintf("text collision scan failed for %s:", filename), shown),
      collapse = "\n  "), call. = FALSE)
  }
  crop_paths <- character()
  if (crops) {
    boxes <- panel_crops(plot, size[[1L]], size[[2L]], units = "in", dpi = dpi)
    crop_paths <- save_panel_crops(plot, boxes, size[[1L]], size[[2L]], units = "in",
      dpi = dpi, bg = bg, out_dir = out_dir)
  }
  list(path = filename, crops = crop_paths)
}
