/**
 * CollectionEditor — Manages dynamic map/array collections.
 *
 * Each entry is rendered as a collapsible card with a header showing
 * the entry key/name, expand/collapse toggle, and delete button.
 * A floating "Add" button creates new entries.
 *
 * For map collections, deletions produce null values in the merge
 * patch (standard JSON Merge Patch deletion). For array collections,
 * edits produce a full-array replacement payload.
 *
 * Exposed as window.CollectionEditor.
 */
window.CollectionEditor = (function () {
  'use strict';

  // ---------------------------------------------------------------------------
  // DOM helpers
  // ---------------------------------------------------------------------------

  function el(tag, className) {
    var node = document.createElement(tag);
    if (className) node.className = className;
    return node;
  }

  // ---------------------------------------------------------------------------
  // Main API
  // ---------------------------------------------------------------------------

  /**
   * Render a collection editor.
   *
   * @param {HTMLElement} container       Where to render the editor.
   * @param {Object}     sectionSchema   The schema section (with collection meta).
   * @param {Object|Array} entries        Current entries.
   *        For map collections: { key: { ...fields } }
   *        For array collections: [ { ...fields } ]
   * @param {Object}     opts
   * @param {Object}     opts.dynamicSources
   * @param {Array}      opts.catalog
   * @param {Function}   opts.onChange     Called when collection changes.
   *        For maps:   onChange(mapPatch)  where mapPatch = { key: value|null }
   *        For arrays: onChange(fullArray)
   * @returns {{ element, getEntries, addEntry, removeEntry }}
   */
  function renderCollection(container, sectionSchema, entries, opts) {
    opts = opts || {};
    var meta = sectionSchema.collection;
    var isMap = meta.kind === 'map';
    var entrySchemaFields = sectionSchema.fields;
    var entrySubsections = sectionSchema.subsections || [];

    // Internal state
    var state = {
      entries: [],         // [{ key, values, widgets, element, expanded }]
      deletedKeys: [],     // keys deleted from map collections
    };

    container.innerHTML = '';

    var entriesContainer = el('div', 'ce-entries');
    container.appendChild(entriesContainer);

    // Add button + inline form
    var addRow = el('div', 'ce-add-row');
    var addBtn = el('button', 'btn btn-primary btn-sm');
    addBtn.type = 'button';
    addBtn.textContent = '+ ' + (meta.add_label || 'Add entry');

    // Inline form (shown instead of prompt)
    var inlineForm = el('div', 'ce-inline-form');
    inlineForm.style.display = 'none';

    var keyInput = el('input', 'text-input ce-inline-key-input');
    keyInput.type = 'text';
    keyInput.placeholder = (meta.key_field && meta.key_field.label) ? meta.key_field.label + ' (unique name)' : 'Name';

    var keyError = el('span', 'ce-inline-key-error');
    keyError.style.display = 'none';

    var inlineConfirmBtn = el('button', 'btn btn-primary btn-sm');
    inlineConfirmBtn.type = 'button';
    inlineConfirmBtn.textContent = 'Create';

    var inlineCancelBtn = el('button', 'btn btn-muted btn-sm');
    inlineCancelBtn.type = 'button';
    inlineCancelBtn.textContent = 'Cancel';

    inlineForm.appendChild(keyInput);
    inlineForm.appendChild(inlineConfirmBtn);
    inlineForm.appendChild(inlineCancelBtn);
    inlineForm.appendChild(keyError);

    function showInlineForm() {
      addBtn.style.display = 'none';
      inlineForm.style.display = '';
      keyInput.value = '';
      keyError.style.display = 'none';
      keyInput.classList.remove('ce-key-input-error');
      keyInput.focus();
    }

    function hideInlineForm() {
      inlineForm.style.display = 'none';
      addBtn.style.display = '';
    }

    function submitInlineForm() {
      var key = keyInput.value.trim();
      if (!key) {
        keyError.textContent = 'Name is required.';
        keyError.style.display = '';
        keyInput.classList.add('ce-key-input-error');
        return;
      }
      if (state.entries.some(function (e) { return e.key === key; })) {
        keyError.textContent = 'An entry with this name already exists.';
        keyError.style.display = '';
        keyInput.classList.add('ce-key-input-error');
        return;
      }
      var defaults = buildDefaults(entrySchemaFields);
      addEntryCard(key, defaults, true);
      fireChange();
      hideInlineForm();
      // Scroll new card into view
      var lastCard = entriesContainer.lastElementChild;
      if (lastCard) lastCard.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    }

    addBtn.addEventListener('click', function () {
      if (isMap && meta.key_field) {
        showInlineForm();
      } else if (!isMap) {
        var defaults2 = buildDefaults(entrySchemaFields);
        addEntryCard(String(state.entries.length), defaults2, true);
        fireChange();
      }
    });

    inlineConfirmBtn.addEventListener('click', submitInlineForm);
    inlineCancelBtn.addEventListener('click', hideInlineForm);
    keyInput.addEventListener('keydown', function (e) {
      if (e.key === 'Enter') { e.preventDefault(); submitInlineForm(); }
      if (e.key === 'Escape') hideInlineForm();
      keyError.style.display = 'none';
      keyInput.classList.remove('ce-key-input-error');
    });

    addRow.appendChild(addBtn);
    addRow.appendChild(inlineForm);
    container.appendChild(addRow);

    // ── Initialize entries ──────────────────────────────────────

    if (isMap && entries && typeof entries === 'object' && !Array.isArray(entries)) {
      Object.keys(entries).forEach(function (key) {
        addEntryCard(key, entries[key], false);
      });
    } else if (!isMap && Array.isArray(entries)) {
      entries.forEach(function (entry, idx) {
        addEntryCard(String(idx), entry, false);
      });
    }

    // ── Entry card ──────────────────────────────────────────────

    function addEntryCard(key, entryValues, expanded) {
      var card = el('div', 'ce-card');
      var entry = {
        key: key,
        values: entryValues || {},
        widgets: {},
        element: card,
        expanded: expanded !== false,
      };

      // Fix secret fields: replace "********" (API mask) with __UNCHANGED__ sentinel
      // so the backend knows these weren't modified.
      entrySchemaFields.forEach(function (fieldSchema) {
        if (fieldSchema.input_type === 'secret') {
          var current = resolveEntryFieldValue(fieldSchema.path, entry.values);
          if (current === '********') {
            setNestedValue(entry.values, fieldSchema.path, '__UNCHANGED__');
          }
        }
      });
      // Also fix secrets in subsections
      entrySubsections.forEach(function (sub) {
        (sub.fields || []).forEach(function (fieldSchema) {
          if (fieldSchema.input_type === 'secret') {
            var current = resolveEntryFieldValue(fieldSchema.path, entry.values);
            if (current === '********') {
              setNestedValue(entry.values, fieldSchema.path, '__UNCHANGED__');
            }
          }
        });
      });

      // Header
      var header = el('div', 'ce-card-header');
      header.setAttribute('role', 'button');
      header.setAttribute('tabindex', '0');
      header.setAttribute('aria-expanded', String(entry.expanded));
      var headerLeft = el('div', 'ce-card-header-left');

      var chevron = el('span', 'sr-chevron');
      chevron.textContent = entry.expanded ? '▾' : '▸';
      headerLeft.appendChild(chevron);

      var keyLabel = el('span', 'ce-card-key');
      keyLabel.textContent = key;
      headerLeft.appendChild(keyLabel);

      // Type badge for providers
      if (isMap && entryValues && entryValues.provider_type) {
        var badge = el('span', 'badge badge-muted ce-type-badge');
        badge.textContent = entryValues.provider_type;
        headerLeft.appendChild(badge);
      }

      // Active badge — shown when this entry is the currently selected provider
      if (opts.activeKey && opts.activeKey === key) {
        var activeBadge = el('span', 'badge badge-success ce-active-badge');
        activeBadge.textContent = 'Active';
        headerLeft.appendChild(activeBadge);
      }

      header.appendChild(headerLeft);

      var headerActions = el('div', 'ce-card-actions');
      var deleteBtn = el('button', 'btn btn-danger btn-sm');
      deleteBtn.type = 'button';
      deleteBtn.textContent = 'Delete';
      deleteBtn.setAttribute('aria-label', 'Delete ' + key);
      deleteBtn.addEventListener('click', function (e) {
        e.stopPropagation();
        if (!window.confirm('Delete "' + key + '"?')) return;
        removeEntryCard(entry);
      });
      headerActions.appendChild(deleteBtn);
      header.appendChild(headerActions);

      header.addEventListener('click', function () {
        entry.expanded = !entry.expanded;
        body.style.display = entry.expanded ? '' : 'none';
        chevron.textContent = entry.expanded ? '▾' : '▸';
        header.setAttribute('aria-expanded', String(entry.expanded));
      });

      header.addEventListener('keydown', function (e) {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          entry.expanded = !entry.expanded;
          body.style.display = entry.expanded ? '' : 'none';
          chevron.textContent = entry.expanded ? '▾' : '▸';
          header.setAttribute('aria-expanded', String(entry.expanded));
        }
      });

      card.appendChild(header);

      // Body
      var body = el('div', 'ce-card-body');
      body.style.display = entry.expanded ? '' : 'none';

      // Render fields for this entry
      entrySchemaFields.forEach(function (fieldSchema) {
        // For credential_refs, _key and _value are the entire entry
        if (fieldSchema.path === '_key' || fieldSchema.path === '_value') {
          // Skip; handled specially for credential_refs
          return;
        }

        var fieldValue = resolveEntryFieldValue(fieldSchema.path, entryValues);
        var widget = window.FormRenderer.renderField(fieldSchema, fieldValue, {
          dynamicSources: opts.dynamicSources,
          catalog: opts.catalog,
          onChange: function (path, newValue) {
            setNestedValue(entry.values, path, newValue);
            fireChange();
          },
          idPrefix: 'ce-' + key,
        });
        entry.widgets[fieldSchema.path] = widget;
        body.appendChild(widget.element);
      });

      // Handle credential_refs special case (key-value pair)
      if (hasCredentialRefFields(entrySchemaFields)) {
        renderCredentialRefFields(entry, body, entryValues, opts);
      }

      // Subsections within the entry
      entrySubsections.forEach(function (sub) {
        var subSection = window.SectionRenderer.renderSection(sub, entryValues, {
          dynamicSources: opts.dynamicSources,
          catalog: opts.catalog,
          onChange: function (path, newValue) {
            setNestedValue(entry.values, path, newValue);
            fireChange();
          },
          startExpanded: false,
          idPrefix: 'ce-' + key + '-' + sub.id,
        });
        body.appendChild(subSection.element);
        // Merge sub-section widgets
        Object.keys(subSection.widgets).forEach(function (p) {
          entry.widgets[p] = subSection.widgets[p];
        });
      });

      card.appendChild(body);
      entriesContainer.appendChild(card);
      state.entries.push(entry);
    }

    function removeEntryCard(entry) {
      var idx = state.entries.indexOf(entry);
      if (idx === -1) return;

      state.entries.splice(idx, 1);
      entry.element.remove();

      if (isMap) {
        state.deletedKeys.push(entry.key);
      }
      fireChange();
    }

    // ── Change notification ─────────────────────────────────────

    function fireChange() {
      if (!opts.onChange) return;

      if (isMap) {
        var patch = {};
        // Deleted keys → null
        state.deletedKeys.forEach(function (k) {
          patch[k] = null;
        });
        // Current entries
        state.entries.forEach(function (e) {
          patch[e.key] = e.values;
        });
        opts.onChange(patch);
      } else {
        // Array: full replacement
        var arr = state.entries.map(function (e) { return e.values; });
        opts.onChange(arr);
      }
    }

    // ── Public API ──────────────────────────────────────────────

    return {
      element: container,
      getEntries: function () {
        if (isMap) {
          var result = {};
          state.entries.forEach(function (e) {
            result[e.key] = e.values;
          });
          return result;
        }
        return state.entries.map(function (e) { return e.values; });
      },
      getPatch: function () {
        if (!isMap) return this.getEntries();
        var patch = {};
        state.deletedKeys.forEach(function (k) {
          patch[k] = null;
        });
        state.entries.forEach(function (e) {
          patch[e.key] = e.values;
        });
        return patch;
      },
      addEntry: function (key, values) {
        addEntryCard(key, values || {}, true);
        fireChange();
      },
      removeEntry: function (key) {
        var entry = state.entries.find(function (e) { return e.key === key; });
        if (entry) removeEntryCard(entry);
      },
      getDeletedKeys: function () {
        return state.deletedKeys.slice();
      },
    };
  }

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  function resolveEntryFieldValue(path, obj) {
    if (!obj || typeof obj !== 'object') return undefined;
    var parts = path.split('.');
    var current = obj;
    for (var i = 0; i < parts.length; i++) {
      if (current == null || typeof current !== 'object') return undefined;
      current = current[parts[i]];
    }
    return current;
  }

  function setNestedValue(obj, path, value) {
    var parts = path.split('.');
    var current = obj;
    for (var i = 0; i < parts.length - 1; i++) {
      if (current[parts[i]] == null || typeof current[parts[i]] !== 'object') {
        current[parts[i]] = {};
      }
      current = current[parts[i]];
    }
    current[parts[parts.length - 1]] = value;
  }

  function buildDefaults(fields) {
    var result = {};
    fields.forEach(function (f) {
      if (f.path === '_key' || f.path === '_value') return;
      if (f.default != null) {
        setNestedValue(result, f.path, JSON.parse(JSON.stringify(f.default)));
      }
    });
    return result;
  }

  function hasCredentialRefFields(fields) {
    return fields.some(function (f) { return f.path === '_key'; }) &&
           fields.some(function (f) { return f.path === '_value'; });
  }

  function renderCredentialRefFields(entry, body, entryValues, opts) {
    // Credential refs use a simple key=value display
    // The key is the entry key (already shown in the header),
    // the value is the secret
    var valueSchema = {
      path: '_value',
      label: 'Credential Value',
      description: 'The credential secret value',
      input_type: 'secret',
      required: true,
      nullable: false,
    };

    var widget = window.FormRenderer.renderField(valueSchema, '', {
      dynamicSources: opts.dynamicSources,
      onChange: function (path, newValue) {
        entry.values = newValue;
        if (opts.onChange) opts.onChange();
      },
    });
    entry.widgets['_value'] = widget;
    body.appendChild(widget.element);
  }

  // ---------------------------------------------------------------------------
  // Public API
  // ---------------------------------------------------------------------------

  return {
    renderCollection: renderCollection,
  };
})();
