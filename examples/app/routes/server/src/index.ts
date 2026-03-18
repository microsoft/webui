// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Express.js API server for the routes example.
 *
 * Resolves state per-route for the Learning Platform.
 * The WebUI dev server proxies to this via --api-port.
 */

import express from 'express';

const app = express();
const PORT = 3100;

// ── Data ─────────────────────────────────────────────────────────

interface Lesson {
  id: string;
  name: string;
  content: string;
}

interface Topic {
  id: string;
  name: string;
  lessons: Lesson[];
}

interface Section {
  id: string;
  name: string;
  icon: string;
  topics: Topic[];
}

const sections: Section[] = [
  {
    id: 'frontend',
    name: 'Frontend',
    icon: '🎨',
    topics: [
      {
        id: 'react',
        name: 'React',
        lessons: [
          { id: 'intro', name: 'Introduction to React', content: 'React is a JavaScript library for building user interfaces.' },
          { id: 'hooks', name: 'React Hooks', content: 'Hooks let you use state and lifecycle features in function components.' },
        ],
      },
      {
        id: 'css',
        name: 'CSS',
        lessons: [
          { id: 'flexbox', name: 'Flexbox Layout', content: 'Flexbox provides a one-dimensional layout model.' },
          { id: 'grid', name: 'CSS Grid', content: 'CSS Grid is a two-dimensional layout system.' },
        ],
      },
    ],
  },
  {
    id: 'backend',
    name: 'Backend',
    icon: '⚙️',
    topics: [
      {
        id: 'rust',
        name: 'Rust',
        lessons: [
          { id: 'ownership', name: 'Ownership', content: "Rust's ownership system ensures memory safety without garbage collection." },
          { id: 'traits', name: 'Traits', content: 'Traits define shared behavior across types.' },
        ],
      },
      {
        id: 'node',
        name: 'Node.js',
        lessons: [
          { id: 'async', name: 'Async Patterns', content: 'Node.js uses an event-driven, non-blocking I/O model.' },
        ],
      },
    ],
  },
  {
    id: 'devops',
    name: 'DevOps',
    icon: '🚀',
    topics: [
      {
        id: 'docker',
        name: 'Docker',
        lessons: [
          { id: 'basics', name: 'Docker Basics', content: 'Docker containers package applications with their dependencies.' },
        ],
      },
    ],
  },
];

function findSection(id: string): Section | undefined {
  return sections.find((s) => s.id === id);
}

function findTopic(section: Section, topicId: string): Topic | undefined {
  return section.topics.find((t) => t.id === topicId);
}

function findLesson(topic: Topic, lessonId: string): Lesson | undefined {
  return topic.lessons.find((l) => l.id === lessonId);
}

// ── Shell state (always included) ────────────────────────────────

function shellState() {
  return {
    title: 'Learning Platform',
    textdirection: 'ltr',
    language: 'en',
    appTitle: 'Learning Platform',
    sections: sections.map(({ id, name, icon }) => ({ id, name, icon })),
  };
}

// ── Route handlers ───────────────────────────────────────────────

app.get('/', (_req, res) => {
  res.json({ state: shellState() });
});

app.get('/sections/:id', (req, res) => {
  const section = findSection(req.params.id);
  if (!section) return res.status(404).json({ state: shellState() });

  res.json({
    state: {
      ...shellState(),
      id: section.id,
      sectionName: section.name,
      sectionIcon: section.icon,
      topics: section.topics.map(({ id, name }) => ({ id, name })),
    },
  });
});

app.get('/sections/:id/topics/:topicId', (req, res) => {
  const section = findSection(req.params.id);
  if (!section) return res.status(404).json({ state: shellState() });
  const topic = findTopic(section, req.params.topicId);
  if (!topic) return res.status(404).json({ state: shellState() });

  res.json({
    state: {
      ...shellState(),
      id: section.id,
      sectionName: section.name,
      sectionIcon: section.icon,
      topicId: topic.id,
      topicName: topic.name,
      topics: section.topics.map(({ id, name }) => ({ id, name })),
      lessons: topic.lessons.map(({ id, name }) => ({ id, name })),
    },
  });
});

app.get('/sections/:id/topics/:topicId/lessons/:lessonId', (req, res) => {
  const section = findSection(req.params.id);
  if (!section) return res.status(404).json({ state: shellState() });
  const topic = findTopic(section, req.params.topicId);
  if (!topic) return res.status(404).json({ state: shellState() });
  const lesson = findLesson(topic, req.params.lessonId);
  if (!lesson) return res.status(404).json({ state: shellState() });

  res.json({
    state: {
      ...shellState(),
      id: section.id,
      sectionName: section.name,
      sectionIcon: section.icon,
      topicId: topic.id,
      topicName: topic.name,
      topics: section.topics.map(({ id, name }) => ({ id, name })),
      lessons: topic.lessons.map(({ id, name }) => ({ id, name })),
      lessonId: lesson.id,
      lessonName: lesson.name,
      lessonContent: lesson.content,
    },
  });
});

app.listen(PORT, () => {
  console.log(`Routes API server running on http://127.0.0.1:${PORT}`);
});
